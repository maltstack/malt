//! Windows HCS layer materialization.
//!
//! Callers must pass only helper-owned paths. This module intentionally knows
//! nothing about OCI references or the daemon protocol: it converts verified,
//! safely extracted layer directories into HCS parent layers and creates a
//! private writable layer over them.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::tier::IsolationError;

const OWNER_MARKER: &str = ".malt-owned-image-store";
const OWNER_MARKER_CONTENT: &[u8] = b"malt-image-store-v1\n";

/// Establish the marker that authorizes later removal below this helper-owned
/// root. Callers must create this root from trusted configuration, never from
/// a protocol field.
pub fn ensure_owned_root(root: &Path) -> Result<(), IsolationError> {
    if !root.is_absolute() {
        return Err(IsolationError::HcsError(
            "owned HCS root must be absolute".to_string(),
        ));
    }
    fs::create_dir_all(root).map_err(IsolationError::IoError)?;
    let marker = root.join(OWNER_MARKER);
    if marker.exists() {
        return verify_owned_root(root);
    }
    fs::write(marker, OWNER_MARKER_CONTENT).map_err(IsolationError::IoError)
}

/// Remove a descendant only after verifying the root marker and rejecting
/// lexical parent escapes and symlink traversal beneath the owned root.
pub fn remove_owned_tree(root: &Path, target: &Path) -> Result<(), IsolationError> {
    verify_owned_root(root)?;
    let relative = target.strip_prefix(root).map_err(|_| {
        IsolationError::HcsError("refusing cleanup outside the helper-owned image root".to_string())
    })?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(IsolationError::HcsError(
            "refusing unsafe helper-owned cleanup path".to_string(),
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        if current.exists()
            && fs::symlink_metadata(&current)
                .map_err(IsolationError::IoError)?
                .file_type()
                .is_symlink()
        {
            return Err(IsolationError::HcsError(
                "refusing cleanup through a symbolic link".to_string(),
            ));
        }
    }
    if target.exists() {
        fs::remove_dir_all(target).map_err(IsolationError::IoError)?;
    }
    Ok(())
}

fn verify_owned_root(root: &Path) -> Result<(), IsolationError> {
    if !root.is_absolute() {
        return Err(IsolationError::HcsError(
            "owned HCS root must be absolute".to_string(),
        ));
    }
    let contents = fs::read(root.join(OWNER_MARKER)).map_err(IsolationError::IoError)?;
    if contents == OWNER_MARKER_CONTENT {
        Ok(())
    } else {
        Err(IsolationError::HcsError(
            "refusing image-store root with an unrecognized owner marker".to_string(),
        ))
    }
}

/// A prepared HCS read-only parent layer. The path is never a protocol value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedLayer {
    pub id: String,
    pub path: PathBuf,
}

/// A session-private writable layer that must be detached and destroyed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableLayer {
    pub path: PathBuf,
    attached: bool,
}

impl WritableLayer {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn attached(&self) -> bool {
        self.attached
    }
}

/// Render the only HCS layer-data JSON MALT supplies. Layer IDs and paths come
/// from the helper-owned prepared registry, not a daemon request.
pub fn layer_data_json(parents: &[PreparedLayer]) -> String {
    json!({
        "SchemaVersion": { "Major": 2, "Minor": 1 },
        "Layers": parents.iter().map(|parent| json!({
            "ID": parent.id,
            "Path": parent.path,
            "PathType": "AbsolutePath",
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

/// Prepare one verified filesystem layer. The first layer is processed as an
/// HCS base OS layer; subsequent layers are imported over the ordered parents.
pub fn materialize_layer(
    destination: &Path,
    source: &Path,
    id: &str,
    parents: &[PreparedLayer],
) -> Result<PreparedLayer, IsolationError> {
    validate_owned_layer_path(destination, id)?;
    if !source.is_dir() {
        return Err(IsolationError::HcsError(format!(
            "verified layer source is not a directory: {}",
            source.display()
        )));
    }
    if destination.exists() {
        return Err(IsolationError::HcsError(format!(
            "refusing to overwrite existing HCS layer: {}",
            destination.display()
        )));
    }
    let result = if parents.is_empty() {
        copy_layer_source(source, destination).and_then(|()| process_base_image(destination))
    } else {
        import_layer(destination, source, parents)
    };
    if let Err(error) = result {
        let _ = remove_owned_directory(destination);
        return Err(error);
    }
    Ok(PreparedLayer {
        id: id.to_string(),
        path: destination.to_path_buf(),
    })
}

/// Create and attach one session-private writable layer over ordered parents.
pub fn initialize_writable_layer(
    destination: &Path,
    parents: &[PreparedLayer],
) -> Result<WritableLayer, IsolationError> {
    validate_owned_layer_path(destination, "workspace")?;
    if parents.is_empty() {
        return Err(IsolationError::HcsError(
            "writable HCS layer requires at least one prepared parent".to_string(),
        ));
    }
    if destination.exists() {
        return Err(IsolationError::HcsError(format!(
            "refusing to reuse existing writable layer: {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| IsolationError::HcsError("writable layer path has no parent".to_string()))?;
    fs::create_dir_all(parent).map_err(IsolationError::IoError)?;
    let data = layer_data_json(parents);
    let result = native::initialize_writable(destination, &data)
        .and_then(|()| native::attach_filter(destination, &data));
    if let Err(error) = result {
        let _ = remove_owned_directory(destination);
        return Err(error);
    }
    Ok(WritableLayer {
        path: destination.to_path_buf(),
        attached: true,
    })
}

/// Detach and destroy an owned writable layer. The caller is responsible for
/// stopping/closing its compute system before invoking this function.
pub fn destroy_writable_layer(workspace: WritableLayer) -> Result<(), IsolationError> {
    if workspace.attached {
        native::detach_filter(&workspace.path)?;
    }
    native::destroy_layer(&workspace.path)?;
    Ok(())
}

/// Destroy an owned read-only prepared layer after no compute system or child
/// layer references it. The helper performs reference checks before this call.
pub fn destroy_prepared_layer(layer: PreparedLayer) -> Result<(), IsolationError> {
    native::destroy_layer(&layer.path)
}

fn validate_owned_layer_path(path: &Path, id: &str) -> Result<(), IsolationError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(IsolationError::HcsError(
            "HCS layer identifier must use ASCII alphanumeric or hyphen characters".to_string(),
        ));
    }
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(IsolationError::HcsError(
            "HCS layer destination must be an absolute owned directory".to_string(),
        ));
    }
    Ok(())
}

fn copy_layer_source(source: &Path, destination: &Path) -> Result<(), IsolationError> {
    fs::create_dir_all(destination).map_err(IsolationError::IoError)?;
    for entry in fs::read_dir(source).map_err(IsolationError::IoError)? {
        let entry = entry.map_err(IsolationError::IoError)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().map_err(IsolationError::IoError)?;
        if file_type.is_symlink() {
            return Err(IsolationError::HcsError(format!(
                "verified Windows layer source contains a symlink: {}",
                source_path.display()
            )));
        }
        if file_type.is_dir() {
            copy_layer_source(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(IsolationError::IoError)?;
        } else {
            return Err(IsolationError::HcsError(format!(
                "verified Windows layer source contains an unsupported entry: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn remove_owned_directory(path: &Path) -> Result<(), IsolationError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(IsolationError::IoError)?;
    }
    Ok(())
}

#[cfg(windows)]
fn process_base_image(path: &Path) -> Result<(), IsolationError> {
    native::process_base_image(path)
}

#[cfg(not(windows))]
fn process_base_image(_path: &Path) -> Result<(), IsolationError> {
    Err(IsolationError::UnsupportedPlatform(
        "HCS layers require Windows".to_string(),
    ))
}

#[cfg(windows)]
fn import_layer(
    destination: &Path,
    source: &Path,
    parents: &[PreparedLayer],
) -> Result<(), IsolationError> {
    native::import_layer(destination, source, &layer_data_json(parents))
}

#[cfg(not(windows))]
fn import_layer(
    _destination: &Path,
    _source: &Path,
    _parents: &[PreparedLayer],
) -> Result<(), IsolationError> {
    Err(IsolationError::UnsupportedPlatform(
        "HCS layers require Windows".to_string(),
    ))
}

#[cfg(windows)]
mod native {
    use std::ffi::{c_void, CString, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError};
    use windows_sys::Win32::System::HostComputeSystem::{
        HcsAttachLayerStorageFilter, HcsDestroyLayer, HcsDetachLayerStorageFilter, HcsImportLayer,
        HcsInitializeWritableLayer,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    use super::IsolationError;

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn checked_hresult(name: &str, result: i32) -> Result<(), IsolationError> {
        if result == 0 {
            Ok(())
        } else {
            Err(IsolationError::HcsError(format!(
                "{name} HRESULT={result:#010x}"
            )))
        }
    }

    pub(super) fn import_layer(
        destination: &Path,
        source: &Path,
        data: &str,
    ) -> Result<(), IsolationError> {
        let destination = wide(destination.as_os_str());
        let source = wide(source.as_os_str());
        let data = wide(OsStr::new(data));
        // SAFETY: each UTF-16 buffer is null terminated and lives for the
        // duration of HcsImportLayer. The caller supplied helper-owned paths.
        checked_hresult("HcsImportLayer", unsafe {
            HcsImportLayer(destination.as_ptr(), source.as_ptr(), data.as_ptr())
        })
    }

    pub(super) fn initialize_writable(
        destination: &Path,
        data: &str,
    ) -> Result<(), IsolationError> {
        let destination = wide(destination.as_os_str());
        let data = wide(OsStr::new(data));
        // SAFETY: buffers are null terminated and valid through the call;
        // null options requests HCS defaults documented for this API.
        checked_hresult("HcsInitializeWritableLayer", unsafe {
            HcsInitializeWritableLayer(destination.as_ptr(), data.as_ptr(), std::ptr::null())
        })
    }

    pub(super) fn attach_filter(destination: &Path, data: &str) -> Result<(), IsolationError> {
        let destination = wide(destination.as_os_str());
        let data = wide(OsStr::new(data));
        // SAFETY: buffers are null terminated and valid through the call.
        checked_hresult("HcsAttachLayerStorageFilter", unsafe {
            HcsAttachLayerStorageFilter(destination.as_ptr(), data.as_ptr())
        })
    }

    pub(super) fn detach_filter(destination: &Path) -> Result<(), IsolationError> {
        let destination = wide(destination.as_os_str());
        // SAFETY: the writable-layer path was created/attached by this module
        // and the buffer is valid for the duration of this call.
        checked_hresult("HcsDetachLayerStorageFilter", unsafe {
            HcsDetachLayerStorageFilter(destination.as_ptr())
        })
    }

    pub(super) fn destroy_layer(destination: &Path) -> Result<(), IsolationError> {
        let destination = wide(destination.as_os_str());
        // SAFETY: the caller passes a helper-owned writable-layer path and the
        // UTF-16 buffer remains valid through HcsDestroyLayer.
        checked_hresult("HcsDestroyLayer", unsafe {
            HcsDestroyLayer(destination.as_ptr())
        })
    }

    pub(super) fn process_base_image(path: &Path) -> Result<(), IsolationError> {
        type ProcessBaseImage = unsafe extern "system" fn(*const u16) -> i32;
        let module_name = wide(OsStr::new("vmcompute.dll"));
        // SAFETY: module_name is valid UTF-16 and null terminated.
        let module = unsafe { LoadLibraryW(module_name.as_ptr()) };
        if module.is_null() {
            return Err(IsolationError::HcsError(format!(
                "LoadLibraryW(vmcompute.dll) failed: {}",
                unsafe { GetLastError() }
            )));
        }
        let result = (|| {
            let symbol = CString::new("ProcessBaseImage").map_err(|error| {
                IsolationError::HcsError(format!("invalid ProcessBaseImage symbol: {error}"))
            })?;
            // SAFETY: module is a live library handle and symbol is a valid C string.
            let pointer =
                unsafe { GetProcAddress(module, symbol.as_ptr().cast()) }.ok_or_else(|| {
                    IsolationError::HcsError(format!(
                        "vmcompute.dll does not export ProcessBaseImage: {}",
                        unsafe { GetLastError() }
                    ))
                })?;
            // SAFETY: ProcessBaseImage is documented by the Windows container
            // runtime with this exact system ABI and `PCWSTR` parameter.
            let process: ProcessBaseImage = unsafe {
                std::mem::transmute::<*const c_void, ProcessBaseImage>(pointer as *const c_void)
            };
            let path = wide(path.as_os_str());
            // SAFETY: path is a null-terminated UTF-16 owned directory path.
            checked_hresult("ProcessBaseImage", unsafe { process(path.as_ptr()) })
        })();
        // SAFETY: module is a successful LoadLibraryW result and is released once after resolving/calling the symbol.
        unsafe { FreeLibrary(module) };
        result
    }
}

#[cfg(not(windows))]
mod native {
    use super::IsolationError;
    use std::path::Path;
    fn unavailable() -> Result<(), IsolationError> {
        Err(IsolationError::UnsupportedPlatform(
            "HCS layers require Windows".to_string(),
        ))
    }
    pub(super) fn initialize_writable(
        _destination: &Path,
        _data: &str,
    ) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn attach_filter(_destination: &Path, _data: &str) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn detach_filter(_destination: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn destroy_layer(_destination: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_data_is_derived_from_prepared_layers() {
        let json = layer_data_json(&[PreparedLayer {
            id: "layer-a".to_string(),
            path: PathBuf::from(r"C:\\MALT\\layers\\a"),
        }]);
        assert!(json.contains("layer-a"));
        assert!(json.contains("\"ID\""));
        assert!(json.contains("AbsolutePath"));
    }

    #[test]
    fn layer_ids_and_relative_destinations_are_refused() {
        assert!(validate_owned_layer_path(Path::new("relative"), "ok").is_err());
        assert!(validate_owned_layer_path(Path::new(r"C:\\MALT\\layers\\ok"), "../bad").is_err());
    }

    #[test]
    fn owned_cleanup_requires_marker_and_stays_below_root() {
        let temporary = tempfile::tempdir().expect("create test root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let target = root.join("prepared").join("image");
        std::fs::create_dir_all(&target).expect("create target");
        assert!(remove_owned_tree(&root, &target).is_err());

        ensure_owned_root(&root).expect("mark owned root");
        remove_owned_tree(&root, &target).expect("remove owned target");
        assert!(!target.exists());
        assert!(remove_owned_tree(&root, &root).is_err());
    }
}
