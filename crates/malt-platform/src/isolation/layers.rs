//! Windows HCS layer materialization.
//!
//! Callers must pass only helper-owned paths. This module intentionally knows
//! nothing about OCI references or the daemon protocol: it converts verified,
//! safely extracted layer directories into HCS parent layers and creates a
//! private writable layer over them.

use std::fs;
use std::path::{Path, PathBuf};

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

/// Reopen a prepared HCS layer from its helper-owned path and derive the exact
/// vmcompute GUID that HCS uses for that path.
pub fn prepared_layer(path: PathBuf) -> Result<PreparedLayer, IsolationError> {
    validate_owned_layer_path(&path)?;
    let id = native::layer_id(&path)?;
    Ok(PreparedLayer { id, path })
}

/// A session-private writable layer that must be detached and destroyed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritableLayer {
    pub path: PathBuf,
    mount_path: String,
    attached: bool,
}

impl WritableLayer {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn attached(&self) -> bool {
        self.attached
    }

    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }
}

/// Prepare one verified filesystem layer. The first layer is processed as an
/// HCS base OS layer; subsequent layers are imported over the ordered parents.
pub fn materialize_layer(
    destination: &Path,
    source: &Path,
    parents: &[PreparedLayer],
) -> Result<PreparedLayer, IsolationError> {
    validate_owned_layer_path(destination)?;
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
    match prepared_layer(destination.to_path_buf()) {
        Ok(layer) => Ok(layer),
        Err(error) => {
            let _ = remove_owned_directory(destination);
            Err(error)
        }
    }
}

/// Create and attach one session-private writable layer over ordered parents.
pub fn initialize_writable_layer(
    destination: &Path,
    parents: &[PreparedLayer],
) -> Result<WritableLayer, IsolationError> {
    validate_owned_layer_path(destination)?;
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
    let result = native::create_sandbox(destination, parents)
        .and_then(|()| native::activate(destination))
        .and_then(|()| native::prepare(destination, parents))
        .and_then(|()| native::mount_path(destination));
    let mount_path = match result {
        Ok(mount_path) => mount_path,
        Err(error) => {
            let _ = remove_owned_directory(destination);
            return Err(error);
        }
    };
    Ok(WritableLayer {
        path: destination.to_path_buf(),
        mount_path,
        attached: true,
    })
}

/// Detach and destroy an owned writable layer. The caller is responsible for
/// stopping/closing its compute system before invoking this function.
pub fn destroy_writable_layer(workspace: WritableLayer) -> Result<(), IsolationError> {
    if workspace.attached {
        native::unprepare(&workspace.path)?;
        native::deactivate(&workspace.path)?;
    }
    native::destroy_layer(&workspace.path)?;
    Ok(())
}

/// Recover a writable layer whose in-memory helper handle was lost (for
/// example, after a helper restart). The path must still be derived by the
/// helper from its owned store; this function accepts no protocol data.
///
/// A recovered workspace was created by [`initialize_writable_layer`], so it
/// must be unprepared and deactivated before its layer is destroyed.
pub fn destroy_recovered_writable_layer(path: &Path) -> Result<(), IsolationError> {
    validate_owned_layer_path(path)?;
    native::unprepare(path)?;
    native::deactivate(path)?;
    native::destroy_layer(path)
}

/// Destroy an owned read-only prepared layer after no compute system or child
/// layer references it. The helper performs reference checks before this call.
pub fn destroy_prepared_layer(layer: PreparedLayer) -> Result<(), IsolationError> {
    native::destroy_layer(&layer.path)
}

fn validate_owned_layer_path(path: &Path) -> Result<(), IsolationError> {
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
    native::import_layer(destination, source, parents)
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

    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    use super::{IsolationError, PreparedLayer};

    #[repr(C)]
    struct DriverInfo {
        flavour: i32,
        home_dir: *const u16,
    }

    #[repr(C)]
    struct LayerDescriptor {
        layer_id: GUID,
        flags: u32,
        path: *const u16,
    }

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

    static EMPTY_DRIVER_HOME: u16 = 0;

    fn driver_info() -> DriverInfo {
        DriverInfo {
            flavour: 1,
            home_dir: &EMPTY_DRIVER_HOME,
        }
    }

    fn with_symbol<T>(
        name: &str,
        invoke: impl FnOnce(*const c_void) -> Result<T, IsolationError>,
    ) -> Result<T, IsolationError> {
        let module_name = wide(OsStr::new("vmcompute.dll"));
        // SAFETY: module_name is valid UTF-16 and null terminated.
        let module = unsafe { LoadLibraryW(module_name.as_ptr()) };
        if module.is_null() {
            return Err(IsolationError::HcsError(format!(
                "LoadLibraryW(vmcompute.dll) failed: {}",
                // SAFETY: GetLastError reads this thread's last-error value.
                unsafe { GetLastError() }
            )));
        }
        let result = (|| {
            let symbol = CString::new(name).map_err(|error| {
                IsolationError::HcsError(format!("invalid vmcompute symbol {name}: {error}"))
            })?;
            // SAFETY: module is live and symbol is a valid C string.
            let pointer =
                unsafe { GetProcAddress(module, symbol.as_ptr().cast()) }.ok_or_else(|| {
                    IsolationError::HcsError(format!(
                        "vmcompute.dll does not export {name}: {}",
                        // SAFETY: GetLastError reads this thread's last-error value.
                        unsafe { GetLastError() }
                    ))
                })?;
            invoke(pointer as *const c_void)
        })();
        // SAFETY: module came from a successful LoadLibraryW call and no symbol
        // pointer escapes this function.
        unsafe { FreeLibrary(module) };
        result
    }

    fn layer_descriptors(
        parents: &[PreparedLayer],
    ) -> Result<(Vec<Vec<u16>>, Vec<LayerDescriptor>), IsolationError> {
        let paths = parents
            .iter()
            .map(|parent| wide(parent.path.as_os_str()))
            .collect::<Vec<_>>();
        let mut descriptors = Vec::with_capacity(parents.len());
        for (parent, path) in parents.iter().zip(&paths) {
            descriptors.push(LayerDescriptor {
                layer_id: layer_guid(&parent.path)?,
                flags: 0,
                path: path.as_ptr(),
            });
        }
        Ok((paths, descriptors))
    }

    fn layer_guid(path: &Path) -> Result<GUID, IsolationError> {
        type NameToGuid = unsafe extern "system" fn(*const u16, *mut GUID) -> i32;
        let path = wide(path.as_os_str());
        with_symbol("NameToGuid", |pointer| {
            // SAFETY: vmcompute exports NameToGuid with this documented system
            // ABI; the transmuted function remains scoped to the loaded module.
            let name_to_guid: NameToGuid = unsafe { std::mem::transmute(pointer) };
            let mut guid = GUID::default();
            // SAFETY: path is null terminated and guid points to writable
            // storage for the duration of the call.
            checked_hresult("NameToGuid", unsafe {
                name_to_guid(path.as_ptr(), &mut guid)
            })?;
            Ok(guid)
        })
    }

    pub(super) fn layer_id(path: &Path) -> Result<String, IsolationError> {
        let guid = layer_guid(path)?;
        Ok(format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        ))
    }

    fn call_layer_operation(name: &'static str, path: &Path) -> Result<(), IsolationError> {
        type LayerOperation = unsafe extern "system" fn(*const DriverInfo, *const u16) -> i32;
        let path = wide(path.as_os_str());
        let info = driver_info();
        with_symbol(name, |pointer| {
            // SAFETY: each selected vmcompute layer operation has this system
            // ABI and the loaded symbol is used only within this closure.
            let operation: LayerOperation = unsafe { std::mem::transmute(pointer) };
            // SAFETY: info and path are valid through the synchronous call.
            checked_hresult(name, unsafe { operation(&info, path.as_ptr()) })
        })
    }

    pub(super) fn import_layer(
        destination: &Path,
        source: &Path,
        parents: &[PreparedLayer],
    ) -> Result<(), IsolationError> {
        type ImportLayer = unsafe extern "system" fn(
            *const DriverInfo,
            *const u16,
            *const u16,
            *const LayerDescriptor,
            usize,
        ) -> i32;
        let destination = wide(destination.as_os_str());
        let source = wide(source.as_os_str());
        let (_paths, descriptors) = layer_descriptors(parents)?;
        let info = driver_info();
        with_symbol("ImportLayer", |pointer| {
            // SAFETY: ImportLayer is exported by vmcompute with this system ABI.
            let import: ImportLayer = unsafe { std::mem::transmute(pointer) };
            // SAFETY: all UTF-16 and descriptor buffers stay valid through this
            // synchronous call; parents are helper-owned prepared layers.
            checked_hresult("ImportLayer", unsafe {
                import(
                    &info,
                    destination.as_ptr(),
                    source.as_ptr(),
                    descriptors.as_ptr(),
                    descriptors.len(),
                )
            })
        })
    }

    pub(super) fn create_sandbox(
        destination: &Path,
        parents: &[PreparedLayer],
    ) -> Result<(), IsolationError> {
        type CreateSandboxLayer = unsafe extern "system" fn(
            *const DriverInfo,
            *const u16,
            usize,
            *const LayerDescriptor,
            usize,
        ) -> i32;
        let destination = wide(destination.as_os_str());
        let (_paths, descriptors) = layer_descriptors(parents)?;
        let info = driver_info();
        with_symbol("CreateSandboxLayer", |pointer| {
            // SAFETY: CreateSandboxLayer is exported by vmcompute with this ABI.
            let create: CreateSandboxLayer = unsafe { std::mem::transmute(pointer) };
            // SAFETY: destination and descriptors remain valid through the call;
            // null parent selects the documented path-based parent descriptors.
            checked_hresult("CreateSandboxLayer", unsafe {
                create(
                    &info,
                    destination.as_ptr(),
                    0,
                    descriptors.as_ptr(),
                    descriptors.len(),
                )
            })
        })
    }

    pub(super) fn activate(path: &Path) -> Result<(), IsolationError> {
        call_layer_operation("ActivateLayer", path)
    }

    pub(super) fn prepare(path: &Path, parents: &[PreparedLayer]) -> Result<(), IsolationError> {
        type PrepareLayer = unsafe extern "system" fn(
            *const DriverInfo,
            *const u16,
            *const LayerDescriptor,
            usize,
        ) -> i32;
        let path = wide(path.as_os_str());
        let (_paths, descriptors) = layer_descriptors(parents)?;
        let info = driver_info();
        with_symbol("PrepareLayer", |pointer| {
            // SAFETY: PrepareLayer is exported by vmcompute with this system ABI.
            let prepare: PrepareLayer = unsafe { std::mem::transmute(pointer) };
            // SAFETY: buffers remain valid through the synchronous call.
            checked_hresult("PrepareLayer", unsafe {
                prepare(
                    &info,
                    path.as_ptr(),
                    descriptors.as_ptr(),
                    descriptors.len(),
                )
            })
        })
    }

    pub(super) fn mount_path(path: &Path) -> Result<String, IsolationError> {
        type GetLayerMountPath =
            unsafe extern "system" fn(*const DriverInfo, *const u16, *mut usize, *mut u16) -> i32;
        let path = wide(path.as_os_str());
        let info = driver_info();
        with_symbol("GetLayerMountPath", |pointer| {
            // SAFETY: GetLayerMountPath is exported by vmcompute with this ABI.
            let get_mount_path: GetLayerMountPath = unsafe { std::mem::transmute(pointer) };
            let mut length = 0usize;
            // SAFETY: null output buffer asks vmcompute for the required length.
            checked_hresult("GetLayerMountPath", unsafe {
                get_mount_path(&info, path.as_ptr(), &mut length, std::ptr::null_mut())
            })?;
            if length == 0 {
                return Err(IsolationError::HcsError(
                    "GetLayerMountPath returned an empty mount path".to_string(),
                ));
            }
            let mut buffer = vec![0u16; length];
            // SAFETY: buffer has the size requested by vmcompute and is valid
            // for the duration of the call.
            checked_hresult("GetLayerMountPath", unsafe {
                get_mount_path(&info, path.as_ptr(), &mut length, buffer.as_mut_ptr())
            })?;
            let end = buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(buffer.len());
            String::from_utf16(&buffer[..end]).map_err(|error| {
                IsolationError::HcsError(format!(
                    "GetLayerMountPath returned invalid UTF-16: {error}"
                ))
            })
        })
    }

    pub(super) fn unprepare(path: &Path) -> Result<(), IsolationError> {
        call_layer_operation("UnprepareLayer", path)
    }

    pub(super) fn deactivate(path: &Path) -> Result<(), IsolationError> {
        call_layer_operation("DeactivateLayer", path)
    }

    pub(super) fn destroy_layer(path: &Path) -> Result<(), IsolationError> {
        call_layer_operation("DestroyLayer", path)
    }

    pub(super) fn process_base_image(path: &Path) -> Result<(), IsolationError> {
        type ProcessBaseImage = unsafe extern "system" fn(*const u16) -> i32;
        let path = wide(path.as_os_str());
        with_symbol("ProcessBaseImage", |pointer| {
            // SAFETY: ProcessBaseImage is documented by the Windows container
            // runtime with this exact system ABI and `PCWSTR` parameter.
            let process: ProcessBaseImage = unsafe { std::mem::transmute(pointer) };
            // SAFETY: path is a null-terminated UTF-16 owned directory path.
            checked_hresult("ProcessBaseImage", unsafe { process(path.as_ptr()) })
        })
    }
}

#[cfg(not(windows))]
mod native {
    use super::{IsolationError, PreparedLayer};
    use std::path::Path;
    fn unavailable<T>() -> Result<T, IsolationError> {
        Err(IsolationError::UnsupportedPlatform(
            "HCS layers require Windows".to_string(),
        ))
    }
    pub(super) fn layer_id(_path: &Path) -> Result<String, IsolationError> {
        unavailable()
    }
    pub(super) fn import_layer(
        _destination: &Path,
        _source: &Path,
        _parents: &[PreparedLayer],
    ) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn create_sandbox(
        _destination: &Path,
        _parents: &[PreparedLayer],
    ) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn activate(_path: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn prepare(_path: &Path, _parents: &[PreparedLayer]) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn mount_path(_path: &Path) -> Result<String, IsolationError> {
        unavailable()
    }
    pub(super) fn unprepare(_path: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn deactivate(_path: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
    pub(super) fn destroy_layer(_path: &Path) -> Result<(), IsolationError> {
        unavailable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_destinations_are_refused() {
        assert!(validate_owned_layer_path(Path::new("relative")).is_err());
        assert!(validate_owned_layer_path(Path::new(r"C:\\MALT\\layers\\ok")).is_ok());
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
