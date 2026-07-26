use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Digest, ImageManifest, Platform};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRecord {
    pub manifest_digest: Digest,
    pub source_reference: String,
    pub platform: Platform,
    pub manifest: ImageManifest,
    pub prepared: bool,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("image store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("image record is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("image record {0} already exists")]
    Exists(Digest),
    #[error("image record {0} does not exist")]
    Missing(Digest),
    #[error("image record digest does not match its requested identity")]
    IdentityMismatch,
}

/// Content-addressed image metadata and blobs. The caller chooses a helper-
/// owned root; this type never treats a user-provided path as an image ID.
#[derive(Debug, Clone)]
pub struct ImageStore { root: PathBuf }

impl ImageStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(root.join("blobs/sha256"))?;
        fs::create_dir_all(root.join("records"))?;
        fs::create_dir_all(root.join("staging"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path { &self.root }

    pub fn blob_path(&self, digest: &Digest) -> PathBuf { self.root.join("blobs/sha256").join(digest.to_string().trim_start_matches("sha256:")) }

    pub fn record_path(&self, digest: &Digest) -> PathBuf { self.root.join("records").join(format!("{}.json", digest.to_string().trim_start_matches("sha256:"))) }

    pub fn staging_path(&self, nonce: &str) -> Result<PathBuf, StoreError> {
        if nonce.is_empty() || !nonce.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') { return Err(StoreError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsafe staging nonce"))); }
        Ok(self.root.join("staging").join(nonce))
    }

    pub fn publish_blob(&self, digest: &Digest, staged: &Path) -> Result<(), StoreError> {
        let destination = self.blob_path(digest);
        if destination.exists() { return Ok(()); }
        let parent = destination.parent().ok_or_else(|| StoreError::Io(std::io::Error::other("MALT blob path has no parent")))?;
        fs::create_dir_all(parent)?;
        atomic_rename(staged, &destination)
    }

    pub fn publish_record(&self, record: &ImageRecord) -> Result<(), StoreError> {
        let destination = self.record_path(&record.manifest_digest);
        if destination.exists() { return Err(StoreError::Exists(record.manifest_digest.clone())); }
        let temporary = destination.with_extension("json.tmp");
        write_new(&temporary, &serde_json::to_vec_pretty(record)?)?;
        atomic_rename(&temporary, &destination)
    }

    pub fn load_record(&self, digest: &Digest) -> Result<ImageRecord, StoreError> {
        let path = self.record_path(digest);
        let mut bytes = Vec::new();
        File::open(&path).map_err(|error| if error.kind() == std::io::ErrorKind::NotFound { StoreError::Missing(digest.clone()) } else { StoreError::Io(error) })?.read_to_end(&mut bytes)?;
        let record: ImageRecord = serde_json::from_slice(&bytes)?;
        if &record.manifest_digest != digest { return Err(StoreError::IdentityMismatch); }
        Ok(record)
    }

    pub fn remove_record(&self, digest: &Digest) -> Result<(), StoreError> {
        let path = self.record_path(digest);
        if !path.exists() { return Err(StoreError::Missing(digest.clone())); }
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn list_records(&self) -> Result<Vec<ImageRecord>, StoreError> {
        let mut records: Vec<ImageRecord> = Vec::new();
        for entry in fs::read_dir(self.root.join("records"))? {
            let entry = entry?;
            if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|extension| extension == "json") {
                let mut bytes = Vec::new();
                File::open(entry.path())?.read_to_end(&mut bytes)?;
                records.push(serde_json::from_slice(&bytes)?);
            }
        }
        records.sort_by(|left, right| left.manifest_digest.to_string().cmp(&right.manifest_digest.to_string()));
        Ok(records)
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn atomic_rename(from: &Path, to: &Path) -> Result<(), StoreError> {
    if to.exists() { return Ok(()); }
    fs::rename(from, to)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Descriptor, ImageReference};

    fn record() -> ImageRecord {
        let digest: Digest = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".parse().expect("digest");
        ImageRecord { manifest_digest: digest.clone(), source_reference: ImageReference::parse("mcr.microsoft.com/windows/nanoserver:ltsc2022").expect("reference").to_string(), platform: Platform { os: "windows".into(), architecture: "amd64".into(), os_version: Some("20348".into()) }, manifest: ImageManifest { schema_version: 2, media_type: None, config: Descriptor { media_type: "config".into(), digest: digest.clone(), size: 1, platform: None }, layers: vec![Descriptor { media_type: "layer".into(), digest, size: 1, platform: None }] }, prepared: false }
    }

    #[test]
    fn records_are_atomically_addressed_by_manifest_digest() {
        let directory = tempfile::tempdir().expect("temp");
        let store = ImageStore::open(directory.path()).expect("store");
        let record = record();
        store.publish_record(&record).expect("publish");
        assert_eq!(store.load_record(&record.manifest_digest).expect("load"), record);
        assert!(matches!(store.publish_record(&record), Err(StoreError::Exists(_))));
    }

    #[test]
    fn corrupt_or_mismatched_records_are_not_accepted() {
        let directory = tempfile::tempdir().expect("temp");
        let store = ImageStore::open(directory.path()).expect("store");
        let record = record();
        std::fs::write(store.record_path(&record.manifest_digest), b"not-json").expect("write corrupt record");
        assert!(matches!(store.load_record(&record.manifest_digest), Err(StoreError::Json(_))));
    }
}
