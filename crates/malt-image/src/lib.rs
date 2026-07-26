//! Verified OCI image acquisition for MALT.
//!
//! This L1 crate owns registry protocol, OCI descriptor validation, safe layer
//! extraction, and content-addressed image records. It deliberately contains
//! no HCS or other OS-specific API calls; materialization belongs in
//! `malt-platform::isolation::layers`.

mod archive;
mod digest;
mod manifest;
mod model;
mod registry;
mod store;

pub use archive::{extract_gzip_layer, ArchiveError};
pub use digest::{verify_reader, Digest, DigestError};
pub use manifest::{parse_image_manifest, select_windows_amd64, ManifestError};
pub use model::{Descriptor, ImageManifest, ImageReference, Platform};
pub use registry::{RegistryClient, RegistryError};
pub use store::{ImageRecord, ImageStore, StoreError};
