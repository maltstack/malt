//! Nonce-based authentication for the elevated helper.
//!
//! The nonce is an 8-byte little-endian u64 read from a file that the helper
//! owns. The daemon must present this nonce in its `ElevateHello` message
//! to prove it has read access to the nonce file (which is restricted to
//! the owning user via directory permissions 0700 / Windows ACLs).

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;

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

    /// Consume `nonce`, returning false if it was already observed.
    pub fn consume(&self, nonce: u64) -> bool {
        let mut seen = match self.seen.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        if !seen.insert(nonce) {
            return false;
        }
        let mut order = match self.order.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };
        order.push_back(nonce);
        if order.len() > self.capacity {
            if let Some(expired) = order.pop_front() {
                seen.remove(&expired);
            }
        }
        true
    }
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
    fn replay_guard_consumes_each_nonce_once() {
        let guard = ReplayGuard::new(2).expect("valid capacity");
        assert!(guard.consume(10));
        assert!(!guard.consume(10));
        assert!(guard.consume(11));
        assert!(guard.consume(12));
        assert!(guard.consume(10), "bounded window releases expired nonce");
    }
}
