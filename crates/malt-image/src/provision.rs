use std::fs::{self, File};

use serde::Deserialize;
use thiserror::Error;

use crate::{
    parse_image_manifest, select_windows_amd64, Descriptor, Digest, ImageManifest, ImageRecord,
    ImageReference, ImageStore, ManifestError, Platform, RegistryClient, RegistryError, StoreError,
};

#[derive(Debug, Error)]
pub enum ProvisionError {
    #[error("invalid image reference: {0}")]
    Reference(String),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("image config is invalid: {0}")]
    Config(#[from] serde_json::Error),
    #[error("selected image is not windows/amd64")]
    UnsupportedPlatform,
    #[error("staging failure: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct ImageConfig {
    #[serde(default)]
    os: String,
    #[serde(default)]
    architecture: String,
    #[serde(rename = "os.version", default)]
    os_version: Option<String>,
}

/// Acquire every selected OCI blob into the caller's owned content-addressed
/// store. It never claims HCS readiness: callers must materialize the layers
/// separately and update the record only after that transaction succeeds.
pub fn acquire_public_windows_image(
    store: &ImageStore,
    reference_text: &str,
) -> Result<ImageRecord, ProvisionError> {
    let reference = ImageReference::parse(reference_text).map_err(ProvisionError::Reference)?;
    let client = RegistryClient::new()?;
    let initial = client.fetch_manifest(&reference)?;
    let (manifest_digest, manifest, platform) = if is_manifest_index(&initial) {
        let selected = select_windows_amd64(&initial, None)?;
        let selected_reference = reference.with_reference(selected.digest.to_string());
        let selected_manifest = client.fetch_manifest_descriptor(&selected_reference, &selected)?;
        let manifest = parse_image_manifest(&selected_manifest)?;
        let platform = selected
            .platform
            .ok_or(ProvisionError::UnsupportedPlatform)?;
        (selected.digest, manifest, platform)
    } else {
        let manifest = parse_image_manifest(&initial)?;
        let (digest, _) = Digest::from_reader(&mut &initial[..])
            .map_err(|error| ProvisionError::Registry(RegistryError::Verify(error)))?;
        let platform = fetch_platform(&client, &reference, &manifest.config)?;
        (digest, manifest, platform)
    };
    if !platform.is_windows_amd64() {
        return Err(ProvisionError::UnsupportedPlatform);
    }
    let staging = store.staging_path(&format!(
        "acquire-{}",
        manifest_digest.to_string().trim_start_matches("sha256:")
    ))?;
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let result = acquire_blobs(&client, &reference, store, &staging, &manifest);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let record = ImageRecord {
        manifest_digest: manifest_digest.clone(),
        source_reference: reference.to_string(),
        platform,
        manifest,
        prepared: false,
    };
    let record = match store.publish_record(&record) {
        Ok(()) => record,
        Err(StoreError::Exists(_)) => store.load_record(&manifest_digest)?,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    };
    let _ = fs::remove_dir_all(staging);
    Ok(record)
}

fn is_manifest_index(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .is_some_and(|value| value.get("manifests").is_some())
}

fn fetch_platform(
    client: &RegistryClient,
    reference: &ImageReference,
    descriptor: &Descriptor,
) -> Result<Platform, ProvisionError> {
    let mut bytes = Vec::new();
    client.download_blob(reference, descriptor, &mut bytes)?;
    let config: ImageConfig = serde_json::from_slice(&bytes)?;
    Ok(Platform {
        os: config.os,
        architecture: config.architecture,
        os_version: config.os_version,
    })
}

fn acquire_blobs(
    client: &RegistryClient,
    reference: &ImageReference,
    store: &ImageStore,
    staging: &std::path::Path,
    manifest: &ImageManifest,
) -> Result<(), ProvisionError> {
    for descriptor in std::iter::once(&manifest.config).chain(manifest.layers.iter()) {
        let staged = staging.join(descriptor.digest.to_string().trim_start_matches("sha256:"));
        let mut output = File::options().create_new(true).write(true).open(&staged)?;
        client.download_blob(reference, descriptor, &mut output)?;
        output.sync_all()?;
        store.publish_blob(&descriptor.digest, &staged)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_manifest_index;

    #[test]
    fn detects_an_index_without_misclassifying_a_single_manifest() {
        assert!(is_manifest_index(br#"{"schemaVersion":2,"manifests":[]}"#));
        assert!(!is_manifest_index(
            br#"{"schemaVersion":2,"config":{},"layers":[]}"#
        ));
        assert!(!is_manifest_index(b"not-json"));
    }
}
