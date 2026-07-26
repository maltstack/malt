use serde::Deserialize;
use thiserror::Error;

use crate::{Descriptor, ImageManifest, Platform};

const OCI_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const OCI_IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const DOCKER_MANIFEST_LIST: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
const DOCKER_MANIFEST: &str = "application/vnd.docker.distribution.manifest.v2+json";

#[derive(Debug, Deserialize)]
struct ManifestIndex {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType")]
    media_type: Option<String>,
    manifests: Vec<Descriptor>,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("manifest schema version {0} is unsupported")]
    SchemaVersion(u32),
    #[error("manifest media type is unsupported: {0}")]
    MediaType(String),
    #[error("manifest index contains no selectable windows/amd64 descriptor")]
    NoWindowsAmd64,
    #[error("manifest index has multiple indistinguishable windows/amd64 descriptors")]
    AmbiguousWindowsAmd64,
    #[error("image manifest has no config descriptor")]
    MissingConfig,
    #[error("image manifest has no filesystem layers")]
    MissingLayers,
}

/// Select exactly one Windows/amd64 manifest descriptor from an OCI index.
pub fn select_windows_amd64(index_json: &[u8], required_os_version: Option<&str>) -> Result<Descriptor, ManifestError> {
    let index: ManifestIndex = serde_json::from_slice(index_json)?;
    if index.schema_version != 2 { return Err(ManifestError::SchemaVersion(index.schema_version)); }
    if let Some(media_type) = index.media_type.as_deref() {
        if media_type != OCI_IMAGE_INDEX && media_type != DOCKER_MANIFEST_LIST {
            return Err(ManifestError::MediaType(media_type.to_string()));
        }
    }
    let candidates = index.manifests.into_iter().filter(|descriptor| {
        descriptor.platform.as_ref().is_some_and(Platform::is_windows_amd64)
            && required_os_version.map(|version| descriptor.platform.as_ref().and_then(|platform| platform.os_version.as_deref()) == Some(version)).unwrap_or(true)
    }).collect::<Vec<_>>();
    match candidates.len() {
        0 => Err(ManifestError::NoWindowsAmd64),
        1 => Ok(candidates.into_iter().next().expect("one candidate")),
        _ => Err(ManifestError::AmbiguousWindowsAmd64),
    }
}

/// Parse and validate a single-platform OCI/Docker manifest.
pub fn parse_image_manifest(manifest_json: &[u8]) -> Result<ImageManifest, ManifestError> {
    let manifest: ImageManifest = serde_json::from_slice(manifest_json)?;
    if manifest.schema_version != 2 { return Err(ManifestError::SchemaVersion(manifest.schema_version)); }
    if let Some(media_type) = manifest.media_type.as_deref() {
        if media_type != OCI_IMAGE_MANIFEST && media_type != DOCKER_MANIFEST {
            return Err(ManifestError::MediaType(media_type.to_string()));
        }
    }
    if manifest.config.size == 0 { return Err(ManifestError::MissingConfig); }
    if manifest.layers.is_empty() || manifest.layers.iter().any(|layer| layer.size == 0) {
        return Err(ManifestError::MissingLayers);
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn select_rejects_linux_only_index() {
        let json = format!(r#"{{"schemaVersion":2,"mediaType":"{OCI_IMAGE_INDEX}","manifests":[{{"mediaType":"{OCI_IMAGE_MANIFEST}","digest":"{DIGEST}","size":3,"platform":{{"os":"linux","architecture":"amd64"}}}}]}}"#);
        assert!(matches!(select_windows_amd64(json.as_bytes(), None), Err(ManifestError::NoWindowsAmd64)));
    }

    #[test]
    fn select_rejects_ambiguous_windows_variants_without_policy() {
        let json = format!(r#"{{"schemaVersion":2,"manifests":[{{"mediaType":"{OCI_IMAGE_MANIFEST}","digest":"{DIGEST}","size":3,"platform":{{"os":"windows","architecture":"amd64","os.version":"20348"}}}},{{"mediaType":"{OCI_IMAGE_MANIFEST}","digest":"{DIGEST}","size":3,"platform":{{"os":"windows","architecture":"amd64","os.version":"26100"}}}}]}}"#);
        assert!(matches!(select_windows_amd64(json.as_bytes(), None), Err(ManifestError::AmbiguousWindowsAmd64)));
        assert_eq!(select_windows_amd64(json.as_bytes(), Some("26100")).expect("select").platform.expect("platform").os_version.as_deref(), Some("26100"));
    }
}
