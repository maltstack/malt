use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("could not read OCI layer archive: {0}")]
    Io(#[from] std::io::Error),
    #[error("OCI layer contains an unsafe path: {0}")]
    UnsafePath(String),
    #[error("OCI layer contains a duplicate path: {0}")]
    DuplicatePath(String),
    #[error("OCI layer contains unsupported non-regular entry: {0}")]
    UnsupportedEntry(String),
}

/// Extract a gzip-compressed OCI layer into a newly created, owned directory.
///
/// The destination must not already exist. Archive links, devices, and paths
/// escaping the destination are refused rather than interpreted.
pub fn extract_gzip_layer(input: impl Read, destination: &Path) -> Result<(), ArchiveError> {
    if destination.exists() {
        return Err(ArchiveError::UnsafePath(
            "destination already exists".to_string(),
        ));
    }
    fs::create_dir_all(destination)?;
    let result = extract_archive(GzDecoder::new(input), destination);
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn extract_archive(reader: impl Read, destination: &Path) -> Result<(), ArchiveError> {
    let mut archive = Archive::new(reader);
    let mut seen = std::collections::BTreeSet::<PathBuf>::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let relative = safe_relative_path(&path)?;
        if !seen.insert(relative.clone()) {
            return Err(ArchiveError::DuplicatePath(relative.display().to_string()));
        }
        let entry_type = entry.header().entry_type();
        let output = destination.join(&relative);
        if entry_type == EntryType::Directory {
            fs::create_dir_all(output)?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(ArchiveError::UnsupportedEntry(
                relative.display().to_string(),
            ));
        }
        let parent = output
            .parent()
            .ok_or_else(|| ArchiveError::UnsafePath(relative.display().to_string()))?;
        fs::create_dir_all(parent)?;
        let mut file = File::options().create_new(true).write(true).open(output)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn safe_relative_path(path: &Path) -> Result<PathBuf, ArchiveError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ArchiveError::UnsafePath(path.display().to_string()));
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => clean.push(value),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(ArchiveError::UnsafePath(path.display().to_string()))
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(ArchiveError::UnsafePath(path.display().to_string()));
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder, Header};

    fn gzip_with_entry(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut compressed = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut archive = Builder::new(&mut compressed);
            let mut header = Header::new_gnu();
            header.set_path(name).expect("path");
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append(&header, bytes).expect("entry");
            archive.finish().expect("finish");
        }
        compressed.finish().expect("gzip")
    }

    #[test]
    fn extract_rejects_path_traversal() {
        let root = tempfile::tempdir().expect("temp");
        assert!(matches!(
            safe_relative_path(Path::new("../outside")),
            Err(ArchiveError::UnsafePath(_))
        ));
        assert!(matches!(
            safe_relative_path(&root.path().join("absolute")),
            Err(ArchiveError::UnsafePath(_))
        ));
    }

    #[test]
    fn extract_regular_file() {
        let root = tempfile::tempdir().expect("temp");
        let bytes = gzip_with_entry("Files/test.txt", b"ok");
        let target = root.path().join("layer");
        extract_gzip_layer(&bytes[..], &target).expect("extract");
        assert_eq!(
            fs::read(target.join("Files/test.txt")).expect("read"),
            b"ok"
        );
    }
}
