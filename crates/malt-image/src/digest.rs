use std::fmt;
use std::io::Read;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// A canonical OCI SHA-256 content digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_reader(reader: &mut impl Read) -> Result<(Self, u64), DigestError> {
        let mut hasher = Sha256::new();
        let mut bytes_read = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| DigestError::Read(error.to_string()))?;
            if read == 0 {
                break;
            }
            bytes_read = bytes_read
                .checked_add(read as u64)
                .ok_or(DigestError::SizeMismatch {
                    expected: u64::MAX,
                    actual: u64::MAX,
                })?;
            hasher.update(&buffer[..read]);
        }
        Ok((Self::from_bytes(hasher.finalize().into()), bytes_read))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Digest {
    type Err = DigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hexadecimal = value
            .strip_prefix("sha256:")
            .ok_or(DigestError::UnsupportedAlgorithm)?;
        if hexadecimal.len() != 64 || !hexadecimal.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DigestError::Malformed);
        }
        let mut output = [0_u8; 32];
        for (index, pair) in hexadecimal.as_bytes().chunks_exact(2).enumerate() {
            let text = std::str::from_utf8(pair).map_err(|_| DigestError::Malformed)?;
            output[index] = u8::from_str_radix(text, 16).map_err(|_| DigestError::Malformed)?;
        }
        Ok(Self(output))
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DigestError {
    #[error("only sha256 OCI digests are supported")]
    UnsupportedAlgorithm,
    #[error("digest must be a canonical sha256:<64 hexadecimal characters> value")]
    Malformed,
    #[error("blob digest mismatch: expected {expected}, received {actual}")]
    Mismatch { expected: Digest, actual: Digest },
    #[error("blob size mismatch: expected {expected} bytes, received {actual} bytes")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("could not read OCI blob: {0}")]
    Read(String),
}

/// Stream and verify a descriptor without retaining its whole body in memory.
pub fn verify_reader(
    reader: &mut impl Read,
    expected: &Digest,
    expected_size: u64,
) -> Result<u64, DigestError> {
    let (actual, bytes_read) = Digest::from_reader(reader)?;
    if bytes_read != expected_size {
        return Err(DigestError::SizeMismatch {
            expected: expected_size,
            actual: bytes_read,
        });
    }
    if &actual != expected {
        return Err(DigestError::Mismatch {
            expected: expected.clone(),
            actual,
        });
    }
    Ok(bytes_read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn canonical_digest_round_trips() {
        let digest: Digest =
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .parse()
                .expect("digest");
        assert_eq!(
            digest.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verification_checks_digest_and_size() {
        let digest: Digest =
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .parse()
                .expect("digest");
        assert_eq!(
            verify_reader(&mut Cursor::new(b"abc"), &digest, 3).expect("verify"),
            3
        );
        assert!(matches!(
            verify_reader(&mut Cursor::new(b"abc"), &digest, 2),
            Err(DigestError::SizeMismatch { .. })
        ));
    }
}
