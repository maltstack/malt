use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Digest;

/// A validated public OCI image reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    pub registry: String,
    pub repository: String,
    pub reference: String,
}

impl ImageReference {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() || value.contains(char::is_whitespace) || value.contains('\0') {
            return Err("image reference must be non-empty and contain no whitespace or NUL".into());
        }
        let (name, reference) = match value.rsplit_once('@') {
            Some((name, digest)) if digest.starts_with("sha256:") => (name, digest),
            Some(_) => return Err("image digest references must use sha256:<hex>".into()),
            None => match value.rsplit_once(':') {
                Some((name, tag)) if !name.rsplit('/').next().unwrap_or_default().contains('.')
                    && !name.rsplit('/').next().unwrap_or_default().contains(':') => (name, tag),
                Some((name, tag)) if name.contains('/') && !tag.contains('/') => (name, tag),
                _ => (value, "latest"),
            },
        };
        let mut components = name.split('/');
        let first = components.next().unwrap_or_default();
        let (registry, repository) = if first.contains('.') || first.contains(':') || first == "localhost" {
            let rest = components.collect::<Vec<_>>().join("/");
            (first.to_ascii_lowercase(), rest)
        } else {
            ("registry-1.docker.io".to_string(), name.to_string())
        };
        let repository = if registry == "registry-1.docker.io" && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository
        };
        if repository.is_empty()
            || repository.split('/').any(|part| part.is_empty() || part == "." || part == "..")
            || !repository.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        {
            return Err("image repository contains an invalid component".into());
        }
        if reference.is_empty() || reference.contains('/') || reference.contains('\0') {
            return Err("image tag or digest is invalid".into());
        }
        Ok(Self { registry, repository, reference: reference.to_string() })
    }

    pub fn manifest_url(&self) -> String {
        format!("https://{}/v2/{}/manifests/{}", self.registry, self.repository, self.reference)
    }

    pub fn blob_url(&self, digest: &Digest) -> String {
        format!("https://{}/v2/{}/blobs/{digest}", self.registry, self.repository)
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.reference.starts_with("sha256:") {
            write!(formatter, "{}/{}@{}", self.registry, self.repository, self.reference)
        } else {
            write!(formatter, "{}/{}:{}", self.registry, self.repository, self.reference)
        }
    }
}

/// The OCI platform chosen from a manifest index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    #[serde(rename = "os.version", default)]
    pub os_version: Option<String>,
}

impl Platform {
    pub fn is_windows_amd64(&self) -> bool {
        self.os.eq_ignore_ascii_case("windows") && self.architecture.eq_ignore_ascii_case("amd64")
    }
}

/// An OCI content descriptor with an already-parsed identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: Digest,
    pub size: u64,
    #[serde(default)]
    pub platform: Option<Platform>,
}

/// A verified single-platform OCI manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: Option<String>,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
}

#[cfg(test)]
mod tests {
    use super::ImageReference;

    #[test]
    fn docker_hub_short_name_is_canonicalized() {
        let reference = ImageReference::parse("hello-world:linux").expect("parse reference");
        assert_eq!(reference.registry, "registry-1.docker.io");
        assert_eq!(reference.repository, "library/hello-world");
        assert_eq!(reference.reference, "linux");
    }

    #[test]
    fn explicit_registry_and_digest_are_preserved() {
        let reference = ImageReference::parse("mcr.microsoft.com/windows/nanoserver@sha256:abcd")
            .expect("parse reference");
        assert_eq!(reference.registry, "mcr.microsoft.com");
        assert_eq!(reference.repository, "windows/nanoserver");
        assert_eq!(reference.reference, "sha256:abcd");
    }
}
