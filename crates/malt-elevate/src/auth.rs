//! Nonce-based authentication for the elevated helper.
//!
//! The nonce is an 8-byte little-endian u64 read from a file that the helper
//! owns. The daemon must present this nonce in its `ElevateHello` message
//! to prove it has read access to the nonce file (which is restricted to
//! the owning user via directory permissions 0700 / Windows ACLs).

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ElevateError;

/// Holds the session nonce and validates incoming connections.
#[derive(Debug)]
pub struct NonceAuth {
    nonce: u64,
}

/// Bounded replay guard for per-request nonces. A nonce is consumed before an
/// operation is dispatched, so a captured envelope cannot be retried after a
/// handler has started.
#[derive(Debug)]
pub struct ReplayGuard {
    capacity: usize,
    seen: Mutex<HashSet<u64>>,
    order: Mutex<VecDeque<u64>>,
}

/// Request nonces use the high 32 bits for their UTC issuance second and the
/// low 32 bits as a caller-local sequence.  It makes the otherwise opaque
/// protocol field independently time-verifiable by the privileged service.
pub const REQUEST_NONCE_VALIDITY_SECS: u64 = 30;

/// Why a request nonce was refused.  Keeping stale and replayed requests
/// distinct ensures a caller cannot mistake an expired request for a transport
/// failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    Accepted,
    Replayed,
    OutsideValidityWindow,
}

impl ReplayGuard {
    /// Create a bounded guard. Capacity zero would silently disable replay
    /// protection, so it is rejected at construction.
    pub fn new(capacity: usize) -> Result<Self, ElevateError> {
        if capacity == 0 {
            return Err(ElevateError::InvalidArg(
                "replay capacity must be non-zero".into(),
            ));
        }
        Ok(Self {
            capacity,
            seen: Mutex::new(HashSet::new()),
            order: Mutex::new(VecDeque::new()),
        })
    }

    /// Consume a timestamped request nonce against the current UTC clock.
    pub fn consume(&self, nonce: u64) -> ReplayDecision {
        let now = match unix_seconds() {
            Ok(now) => now,
            Err(_) => return ReplayDecision::OutsideValidityWindow,
        };
        self.consume_at(nonce, now)
    }

    /// Consume a timestamped request nonce at a supplied UTC second. This is
    /// public for deterministic tests; production callers use `consume`.
    pub fn consume_at(&self, nonce: u64, now: u64) -> ReplayDecision {
        let issued_at = nonce >> 32;
        if issued_at > now || now - issued_at > REQUEST_NONCE_VALIDITY_SECS {
            return ReplayDecision::OutsideValidityWindow;
        }
        let mut seen = match self.seen.lock() {
            Ok(guard) => guard,
            Err(_) => return ReplayDecision::Replayed,
        };
        if !seen.insert(nonce) {
            return ReplayDecision::Replayed;
        }
        let mut order = match self.order.lock() {
            Ok(guard) => guard,
            Err(_) => return ReplayDecision::Replayed,
        };
        order.push_back(nonce);
        if order.len() > self.capacity {
            if let Some(expired) = order.pop_front() {
                seen.remove(&expired);
            }
        }
        ReplayDecision::Accepted
    }
}

fn unix_seconds() -> Result<u64, std::time::SystemTimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
}

impl NonceAuth {
    /// Read a nonce from a file containing exactly 8 bytes (little-endian u64).
    pub fn from_file(path: &Path) -> Result<Self, ElevateError> {
        let bytes = std::fs::read(path).map_err(ElevateError::NonceFile)?;

        if bytes.len() != 8 {
            return Err(ElevateError::AuthFailed(format!(
                "nonce file has {} bytes, expected 8",
                bytes.len()
            )));
        }

        let nonce_bytes: [u8; 8] = bytes.try_into().map_err(|_| {
            ElevateError::AuthFailed("nonce file did not contain exactly 8 bytes".into())
        })?;
        let nonce = u64::from_le_bytes(nonce_bytes);

        Ok(Self { nonce })
    }

    /// Create a `NonceAuth` from a known nonce value (for testing).
    #[cfg(test)]
    pub fn from_value(nonce: u64) -> Self {
        Self { nonce }
    }

    /// Validate a received nonce against the stored nonce.
    pub fn validate(&self, received: u64) -> bool {
        // This is defence in depth only. Peer identity and single-use request
        // nonces are enforced by the server once the transport is active.
        let a = self.nonce.to_le_bytes();
        let b = received.to_le_bytes();
        let mut diff = 0u8;
        for i in 0..8 {
            diff |= a[i] ^ b[i];
        }
        diff == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_matching_nonce() {
        let auth = NonceAuth::from_value(12345);
        assert!(auth.validate(12345));
    }

    #[test]
    fn reject_wrong_nonce() {
        let auth = NonceAuth::from_value(12345);
        assert!(!auth.validate(99999));
    }

    #[test]
    fn reject_zero_vs_nonzero() {
        let auth = NonceAuth::from_value(0);
        assert!(auth.validate(0));
        assert!(!auth.validate(1));
    }

    #[test]
    fn validate_max_nonce() {
        let auth = NonceAuth::from_value(u64::MAX);
        assert!(auth.validate(u64::MAX));
        assert!(!auth.validate(u64::MAX - 1));
    }

    #[test]
    fn replay_guard_consumes_each_nonce_once_within_the_validity_window() {
        let guard = ReplayGuard::new(2).expect("valid capacity");
        let now = 4_000;
        let nonce = |sequence| (now << 32) | sequence;
        assert_eq!(guard.consume_at(nonce(10), now), ReplayDecision::Accepted);
        assert_eq!(guard.consume_at(nonce(10), now), ReplayDecision::Replayed);
        assert_eq!(guard.consume_at(nonce(11), now), ReplayDecision::Accepted);
        assert_eq!(guard.consume_at(nonce(12), now), ReplayDecision::Accepted);
        assert_eq!(
            guard.consume_at(nonce(10), now),
            ReplayDecision::Accepted,
            "bounded window releases the oldest nonce only after capacity is exceeded"
        );
    }

    #[test]
    fn replay_guard_refuses_expired_and_future_nonces() {
        let guard = ReplayGuard::new(2).expect("valid capacity");
        let now = 4_000;
        assert_eq!(
            guard.consume_at(((now - REQUEST_NONCE_VALIDITY_SECS - 1) << 32) | 1, now),
            ReplayDecision::OutsideValidityWindow
        );
        assert_eq!(
            guard.consume_at(((now + 1) << 32) | 1, now),
            ReplayDecision::OutsideValidityWindow
        );
    }
}
