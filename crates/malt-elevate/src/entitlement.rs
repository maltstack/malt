//! Helper-owned daemon enrollment evidence.
//!
//! The service owns this registry in memory. A service restart deliberately
//! invalidates every enrollment, requiring a fresh explicit UAC approval;
//! stale process claims are never restored from an unprivileged store.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use malt_platform::ipc::{process_identity, PeerIdentity, ProcessIdentity};

use crate::error::ElevateError;

/// One daemon process explicitly approved by an elevated operator.
#[derive(Debug, Clone)]
pub struct EnrolledDaemon {
    identity: ProcessIdentity,
}

/// In-memory authority held solely by the privileged helper service.
#[derive(Debug, Default)]
pub struct EnrollmentRegistry {
    daemons: HashMap<u32, EnrolledDaemon>,
    sessions: HashMap<u32, SessionEntitlement>,
}

#[derive(Debug, Clone)]
struct SessionEntitlement {
    owner: String,
    storage_root: PathBuf,
    pids: HashMap<u32, ProcessIdentity>,
}

impl EnrollmentRegistry {
    /// Enrol `target_pid` only when the request itself came from an elevated
    /// peer of the same Windows principal. The PID's observed identity—not a
    /// caller-supplied image, SID, or creation time—is what is recorded.
    pub fn enroll(
        &mut self,
        requester: &PeerIdentity,
        requester_elevated: bool,
        target_pid: u32,
    ) -> Result<(), ElevateError> {
        if !requester_elevated {
            return Err(ElevateError::AuthFailed(
                "daemon enrollment requires an explicitly elevated requester".to_string(),
            ));
        }
        let identity = process_identity(target_pid).map_err(ElevateError::Connection)?;
        if identity.principal != requester.principal {
            return Err(ElevateError::AuthFailed(format!(
                "refused enrollment of PID {target_pid}: its principal does not match the UAC-approved requester"
            )));
        }
        if let Some(existing) = self.daemons.get(&target_pid) {
            if existing.identity != identity {
                return Err(ElevateError::AuthFailed(format!(
                    "refused enrollment of PID {target_pid}: PID identity changed"
                )));
            }
            return Ok(());
        }
        self.daemons.insert(target_pid, EnrolledDaemon { identity });
        Ok(())
    }

    /// Re-observe the connected client process and return true only if all
    /// recorded identity evidence still matches. PID reuse and executable
    /// replacement therefore revoke access instead of inheriting it.
    pub fn is_currently_enrolled(&mut self, peer: &PeerIdentity) -> Result<bool, ElevateError> {
        let Some(record) = self.daemons.get(&peer.process_id) else {
            return Ok(false);
        };
        let observed = process_identity(peer.process_id).map_err(ElevateError::Connection)?;
        if observed == record.identity {
            Ok(true)
        } else {
            self.daemons.remove(&peer.process_id);
            Ok(false)
        }
    }

    /// Register a session's only permitted filesystem root and process set.
    /// The caller may name resources, but the helper observes their identity
    /// and refuses them unless they belong to the enrolled daemon's principal.
    pub fn register_session(
        &mut self,
        peer: &PeerIdentity,
        session_id: u32,
        storage_root: &str,
        pids: &[u32],
    ) -> Result<(), ElevateError> {
        if !self.is_currently_enrolled(peer)? {
            return Err(ElevateError::AuthFailed(
                "session registration requires an enrolled daemon process".to_string(),
            ));
        }
        let owner = peer.principal.clone();
        let storage_root = malt_platform::fs::canonicalize_path(Path::new(storage_root))
            .map_err(ElevateError::Connection)?;
        let mut observed_pids = HashMap::new();
        for pid in pids {
            let identity = process_identity(*pid).map_err(ElevateError::Connection)?;
            if identity.principal != owner {
                return Err(ElevateError::AuthFailed(format!(
                    "PID {pid} does not belong to the enrolled daemon principal"
                )));
            }
            observed_pids.insert(*pid, identity);
        }
        self.sessions.insert(
            session_id,
            SessionEntitlement {
                owner,
                storage_root,
                pids: observed_pids,
            },
        );
        Ok(())
    }

    /// Validate a path using the service's canonical session root.
    pub fn allows_path(
        &self,
        peer: &PeerIdentity,
        session_id: u32,
        path: &Path,
    ) -> Result<bool, ElevateError> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Ok(false);
        };
        if session.owner != peer.principal {
            return Ok(false);
        }
        malt_platform::fs::canonical_path_within(&session.storage_root, path)
            .map_err(ElevateError::Connection)
    }

    /// Validate an observed PID against the session record and reject PID reuse.
    pub fn allows_pid(
        &mut self,
        peer: &PeerIdentity,
        session_id: u32,
        pid: u32,
    ) -> Result<bool, ElevateError> {
        let Some(session) = self.sessions.get(&session_id) else {
            return Ok(false);
        };
        if session.owner != peer.principal {
            return Ok(false);
        }
        let Some(recorded) = session.pids.get(&pid) else {
            return Ok(false);
        };
        let observed = process_identity(pid).map_err(ElevateError::Connection)?;
        Ok(observed == *recorded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_elevated_enrollment_is_refused_before_pid_inspection() {
        let mut registry = EnrollmentRegistry::default();
        let requester = PeerIdentity {
            process_id: 1,
            principal: "S-1-5-21-test".to_string(),
        };
        assert!(registry.enroll(&requester, false, 1).is_err());
    }
}
