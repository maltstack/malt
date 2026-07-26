//! Nonce-based authentication for the elevated helper.
//!
//! The nonce is an 8-byte little-endian u64 read from a file that the helper
//! owns. The daemon must present this nonce in its `ElevateHello` message
//! to prove it has read access to the nonce file (which is restricted to
//! the owning user via directory permissions 0700 / Windows ACLs).

use std::path::Path;

use crate::error::ElevateError;

/// Holds the session nonce and validates incoming connections.
#[derive(Debug)]
pub struct NonceAuth {
    nonce: u64,
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
}
