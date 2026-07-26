//! Helper-owned daemon enrollment evidence.
//!
//! The service owns this registry in memory. A service restart deliberately
//! invalidates every enrollment, requiring a fresh explicit UAC approval;
//! stale process claims are never restored from an unprivileged store.

use std::collections::HashMap;

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
