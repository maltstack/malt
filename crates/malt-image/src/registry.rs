use std::collections::BTreeMap;
use std::io::Read;

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::StatusCode;
use thiserror::Error;

use crate::{verify_reader, Descriptor, ImageReference};

const ACCEPT_MANIFESTS: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("registry request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("registry refused access without a supported bearer challenge")]
    UnsupportedChallenge,
    #[error("registry bearer challenge is malformed")]
    MalformedChallenge,
    #[error("registry bearer-token response has no token")]
    MissingToken,
    #[error("registry returned unexpected status {0}")]
    Status(StatusCode),
    #[error("registry content-length {actual} disagrees with descriptor size {expected}")]
    ContentLength { expected: u64, actual: u64 },
    #[error("registry blob verification failed: {0}")]
    Verify(#[from] crate::DigestError),
    #[error("registry body read failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Minimal OCI Distribution API client for anonymous public registries.
#[derive(Debug, Clone)]
pub struct RegistryClient {
    client: Client,
}

impl RegistryClient {
    pub fn new() -> Result<Self, RegistryError> {
        Ok(Self { client: Client::builder().user_agent("malt-image/0.1").build()? })
    }

    pub fn fetch_manifest(&self, reference: &ImageReference) -> Result<Vec<u8>, RegistryError> {
        let response = self.get_with_bearer(&reference.manifest_url(), Some(ACCEPT_MANIFESTS))?;
        read_success(response)
    }

    /// Fetch a manifest selected by an index and verify it against that
    /// index's descriptor before the caller parses or publishes it.
    pub fn fetch_manifest_descriptor(&self, reference: &ImageReference, descriptor: &Descriptor) -> Result<Vec<u8>, RegistryError> {
        let response = self.get_with_bearer(&reference.manifest_url(), Some(ACCEPT_MANIFESTS))?;
        let bytes = read_success(response)?;
        if bytes.len() as u64 != descriptor.size {
            return Err(RegistryError::ContentLength { expected: descriptor.size, actual: bytes.len() as u64 });
        }
        crate::verify_reader(&mut &bytes[..], &descriptor.digest, descriptor.size)?;
        Ok(bytes)
    }

    /// Download a descriptor to a caller-supplied writer while checking both
    /// declared size and SHA-256. Callers publish the output only after this
    /// returns success.
    pub fn download_blob(&self, reference: &ImageReference, descriptor: &Descriptor, output: &mut impl std::io::Write) -> Result<u64, RegistryError> {
        let mut response = self.get_with_bearer(&reference.blob_url(&descriptor.digest), None)?;
        if let Some(content_length) = response.content_length() {
            if content_length != descriptor.size { return Err(RegistryError::ContentLength { expected: descriptor.size, actual: content_length }); }
        }
        let mut verifier = TeeReader { source: &mut response, target: output };
        verify_reader(&mut verifier, &descriptor.digest, descriptor.size).map_err(RegistryError::Verify)
    }

    fn get_with_bearer(&self, url: &str, accept: Option<&str>) -> Result<Response, RegistryError> {
        let initial = self.request(url, accept, None)?;
        if initial.status().is_success() { return Ok(initial); }
        if initial.status() != StatusCode::UNAUTHORIZED { return Err(RegistryError::Status(initial.status())); }
        let header = initial.headers().get(WWW_AUTHENTICATE).and_then(|value| value.to_str().ok()).ok_or(RegistryError::UnsupportedChallenge)?;
        let token_url = bearer_token_url(header)?;
        let token_response = self.client.get(token_url).send()?.error_for_status()?;
        let token: BearerToken = token_response.json()?;
        let token = token.token()?;
        let response = self.request(url, accept, Some(&token))?;
        if response.status().is_success() { Ok(response) } else { Err(RegistryError::Status(response.status())) }
    }

    fn request(&self, url: &str, accept: Option<&str>, token: Option<&str>) -> Result<Response, RegistryError> {
        let mut request = self.client.get(url);
        if let Some(accept) = accept { request = request.header(ACCEPT, accept); }
        if let Some(token) = token { request = request.header(AUTHORIZATION, format!("Bearer {token}")); }
        Ok(request.send()?)
    }
}

#[derive(Debug, serde::Deserialize)]
struct BearerToken {
    token: Option<String>,
    access_token: Option<String>,
}

impl BearerToken {
    fn token(self) -> Result<String, RegistryError> { self.token.or(self.access_token).ok_or(RegistryError::MissingToken) }
}

fn bearer_token_url(header: &str) -> Result<String, RegistryError> {
    let values = header.strip_prefix("Bearer ").ok_or(RegistryError::UnsupportedChallenge)?;
    let mut parameters = BTreeMap::new();
    for piece in values.split(',') {
        let (key, value) = piece.trim().split_once('=').ok_or(RegistryError::MalformedChallenge)?;
        parameters.insert(key.trim(), value.trim().trim_matches('"'));
    }
    let realm = parameters.get("realm").ok_or(RegistryError::MalformedChallenge)?;
    if !realm.starts_with("https://") { return Err(RegistryError::MalformedChallenge); }
    let query = parameters.iter().filter(|(key, _)| **key != "realm").map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value))).collect::<Vec<_>>().join("&");
    Ok(if query.is_empty() { (*realm).to_string() } else { format!("{realm}?{query}") })
}

fn percent_encode(value: &str) -> String {
    value.bytes().flat_map(|byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') { vec![(byte as char).to_string()] }
        else { vec![format!("%{byte:02X}")] }
    }).collect()
}

fn read_success(mut response: Response) -> Result<Vec<u8>, RegistryError> {
    if !response.status().is_success() { return Err(RegistryError::Status(response.status())); }
    let mut output = Vec::new();
    response.read_to_end(&mut output)?;
    Ok(output)
}

struct TeeReader<'a, R, W> { source: &'a mut R, target: &'a mut W }

impl<R: Read, W: std::io::Write> Read for TeeReader<'_, R, W> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.source.read(buffer)?;
        self.target.write_all(&buffer[..count])?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_challenge_preserves_service_and_scope() {
        let url = bearer_token_url(r#"Bearer realm="https://auth.example/token",service="registry.example",scope="repository:demo/image:pull""#).expect("parse");
        assert_eq!(url, "https://auth.example/token?scope=repository%3Ademo%2Fimage%3Apull&service=registry.example");
    }
}
