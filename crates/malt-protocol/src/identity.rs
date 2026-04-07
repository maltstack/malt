//! BLAKE3 content-addressed identity for the VNP wire protocol.
//!
//! Provides a cryptographic fingerprint of arbitrary data that can be attached
//! to VNP envelopes for integrity verification and peer identification.

use blake3::Hasher;

/// A 32-byte BLAKE3 hash used as a content-addressed wire identity.
pub type IdentityHash = [u8; 32];

/// BLAKE3 content-addressed identity for wire protocol messages.
///
/// Each identity binds a hash of the message payload to a monotonic timestamp,
/// enabling replay detection and content verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireIdentity {
    /// BLAKE3 hash of the content.
    pub hash: IdentityHash,
    /// Unix timestamp (seconds) when the identity was created.
    pub timestamp: u64,
}

impl WireIdentity {
    /// Compute a new `WireIdentity` from the given data.
    ///
    /// The timestamp is captured from `std::time::SystemTime` at call time.
    pub fn from_data(data: &[u8]) -> Self {
        let hash = Self::hash_bytes(data);
        let timestamp = current_timestamp_secs();
        Self { hash, timestamp }
    }

    /// Compute a `WireIdentity` with an explicit timestamp (useful for tests).
    pub fn from_data_with_timestamp(data: &[u8], timestamp: u64) -> Self {
        let hash = Self::hash_bytes(data);
        Self { hash, timestamp }
    }

    /// Verify that the given data matches this identity's hash.
    pub fn verify(&self, data: &[u8]) -> bool {
        self.hash == Self::hash_bytes(data)
    }

    /// Compute the BLAKE3 hash of the given bytes.
    pub fn hash_bytes(data: &[u8]) -> IdentityHash {
        let mut hasher = Hasher::new();
        hasher.update(data);
        *hasher.finalize().as_bytes()
    }

    /// Return the hash as a hex-encoded string.
    pub fn hash_hex(&self) -> String {
        hex_encode(&self.hash)
    }
}

fn current_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes_deterministic() {
        let data = b"hello world";
        let h1 = WireIdentity::hash_bytes(data);
        let h2 = WireIdentity::hash_bytes(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_bytes_different_data() {
        let h1 = WireIdentity::hash_bytes(b"hello");
        let h2 = WireIdentity::hash_bytes(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_from_data_produces_hash() {
        let data = b"test payload";
        let identity = WireIdentity::from_data_with_timestamp(data, 1_000_000);
        assert_eq!(identity.timestamp, 1_000_000);
        assert!(identity.verify(data));
    }

    #[test]
    fn test_verify_succeeds_for_matching_data() {
        let data = b"important message";
        let identity = WireIdentity::from_data_with_timestamp(data, 42);
        assert!(identity.verify(data));
    }

    #[test]
    fn test_verify_fails_for_tampered_data() {
        let data = b"original";
        let identity = WireIdentity::from_data_with_timestamp(data, 42);
        assert!(!identity.verify(b"tampered"));
    }

    #[test]
    fn test_verify_fails_for_empty_when_data_nonempty() {
        let data = b"not empty";
        let identity = WireIdentity::from_data_with_timestamp(data, 42);
        assert!(!identity.verify(b""));
    }

    #[test]
    fn test_hash_hex_format() {
        let data = b"abc";
        let identity = WireIdentity::from_data_with_timestamp(data, 0);
        let hex = identity.hash_hex();
        assert_eq!(hex.len(), 64); // 32 bytes = 64 hex chars
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_empty_data_hash() {
        let identity = WireIdentity::from_data_with_timestamp(b"", 0);
        // BLAKE3 of empty input is a known constant — just verify it's consistent
        assert!(identity.verify(b""));
        assert!(!identity.verify(b"not empty"));
    }

    #[test]
    fn test_timestamp_is_set_from_system_time() {
        let before = current_timestamp_secs();
        let identity = WireIdentity::from_data(b"timing test");
        let after = current_timestamp_secs();
        assert!(identity.timestamp >= before);
        assert!(identity.timestamp <= after);
    }

    #[test]
    fn test_identity_equality() {
        let data = b"same data";
        let a = WireIdentity::from_data_with_timestamp(data, 100);
        let b = WireIdentity::from_data_with_timestamp(data, 100);
        assert_eq!(a, b);
    }

    #[test]
    fn test_identity_inequality_different_timestamp() {
        let data = b"same data";
        let a = WireIdentity::from_data_with_timestamp(data, 100);
        let b = WireIdentity::from_data_with_timestamp(data, 200);
        assert_ne!(a, b);
    }

    #[test]
    fn test_identity_clone() {
        let identity = WireIdentity::from_data_with_timestamp(b"clone me", 999);
        let cloned = identity.clone();
        assert_eq!(identity, cloned);
    }
}
