//! Windows HCS (Host Compute System) container support.
//!
//! HCS is the same API Windows containers and WSL2 are built on. It provides
//! much stronger isolation than Job Objects (Contained tier vs Capped), at
//! the cost of requiring the Windows Containers optional feature to be
//! installed on the host.
//!
//! Real HCS calls are gated behind the `hcs` Cargo feature (off by default)
//! since they require that host feature. `hcs_available()` is a cheap,
//! always-on runtime check (looks for `computecore.dll`) usable regardless
//! of the feature flag, so callers can report real capability even on
//! builds that don't compile the native backend in.
//!
//! A fake-mode backend (enabled via `MALT_HCS_FAKE=1`) lets higher layers
//! exercise the compute-system lifecycle in tests without a real HCS host.

use std::path::PathBuf;

use super::tier::IsolationError;

/// Configuration for creating an HCS compute system.
#[derive(Debug, Clone)]
pub struct HcsConfig {
    pub id: String,
    /// HCS compute system configuration document, as JSON.
    pub config_json: String,
}

/// Parameters for launching a process inside a compute system.
#[derive(Debug, Clone, Default)]
pub struct HcsProcessParameters {
    pub application_name: Option<String>,
    pub command_line: String,
    pub working_directory: Option<String>,
    pub environment: Vec<(String, String)>,
}

/// Handle to a running HCS compute system.
#[derive(Debug)]
pub struct HcsComputeSystem {
    handle: isize,
    pub id: String,
}

impl HcsComputeSystem {
    pub fn raw_handle(&self) -> isize {
        self.handle
    }
}

/// Handle to a process running inside a compute system.
#[derive(Debug)]
pub struct HcsProcess {
    handle: isize,
    pub process_id: u32,
}

impl HcsProcess {
    pub fn raw_handle(&self) -> isize {
        self.handle
    }
}

/// Result of launching a process, including its I/O pipe handles if requested.
#[derive(Debug)]
pub struct HcsProcessLaunch {
    pub process: HcsProcess,
    pub stdin_handle: Option<isize>,
    pub stdout_handle: Option<isize>,
    pub stderr_handle: Option<isize>,
}

/// Cheap, always-on check for whether HCS is available on this machine.
///
/// This does not require the `hcs` feature — it only checks whether
/// `computecore.dll` is present, so capability probing can report accurate
/// results even on builds that don't compile the native backend in.
pub fn hcs_available() -> bool {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let compute_core = PathBuf::from(system_root)
        .join("System32")
        .join("computecore.dll");
    compute_core.exists()
}

fn hcs_feature_enabled() -> bool {
    cfg!(feature = "hcs")
}

fn hcs_fake_mode_enabled() -> bool {
    hcs_fake_mode_from_env(std::env::var("MALT_HCS_FAKE").ok().as_deref())
}

fn hcs_fake_mode_from_env(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.to_ascii_lowercase()),
        Some(v) if v == "1" || v == "true" || v == "yes"
    )
}

/// Verify the HCS backend is actually usable before attempting real calls:
/// the `hcs` feature must be compiled in, `computecore.dll` must be present,
/// and the specific symbols this module calls must resolve.
pub fn ensure_hcs_runtime() -> Result<(), IsolationError> {
    if hcs_fake_mode_enabled() {
        return Ok(());
    }
    if !hcs_feature_enabled() {
        return Err(IsolationError::HcsError(
            "HCS backend requires building malt-platform with the `hcs` feature".to_string(),
        ));
    }
    if !hcs_available() {
        return Err(IsolationError::HcsError(
            "computecore.dll not found (Windows Containers feature may not be installed)"
                .to_string(),
        ));
    }
    #[cfg(feature = "hcs")]
    {
        raw::HcsDll::load()
            .and_then(|dll| dll.probe_required_symbols())
            .map_err(IsolationError::HcsError)?;
    }
    Ok(())
}

fn validate_hcs_config(config: &HcsConfig) -> Result<(), IsolationError> {
    if config.id.trim().is_empty() {
        return Err(IsolationError::HcsError(
            "compute system id cannot be empty".to_string(),
        ));
    }
    if config.config_json.trim().is_empty() {
        return Err(IsolationError::HcsError(
            "compute system config_json cannot be empty".to_string(),
        ));
    }
    serde_json::from_str::<serde_json::Value>(&config.config_json).map_err(|error| {
        IsolationError::HcsError(format!(
            "compute system config_json is not valid JSON: {error}"
        ))
    })?;
    Ok(())
}

/// Create and start a new HCS compute system.
pub fn create_compute_system(config: &HcsConfig) -> Result<HcsComputeSystem, IsolationError> {
    validate_hcs_config(config)?;
    if hcs_fake_mode_enabled() {
        return Ok(fake::create_compute_system(config));
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::create_compute_system(config)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Open a handle to an already-running compute system by id.
pub fn open_compute_system(id: &str) -> Result<HcsComputeSystem, IsolationError> {
    if id.trim().is_empty() {
        return Err(IsolationError::HcsError(
            "compute system id cannot be empty".to_string(),
        ));
    }
    if hcs_fake_mode_enabled() {
        return fake::open_compute_system(id);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::open_compute_system(id)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Terminate a compute system and release its handle.
pub fn terminate_compute_system(handle: isize) -> Result<(), IsolationError> {
    if hcs_fake_mode_enabled() {
        return fake::terminate_compute_system(handle);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::terminate_compute_system(handle)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Launch a process inside a compute system.
pub fn create_process(
    compute_system: isize,
    params: &HcsProcessParameters,
) -> Result<HcsProcessLaunch, IsolationError> {
    if params.command_line.trim().is_empty() {
        return Err(IsolationError::HcsError(
            "process command_line cannot be empty".to_string(),
        ));
    }
    if hcs_fake_mode_enabled() {
        return fake::create_process(compute_system, params);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::create_process(compute_system, params)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Block until a process inside a compute system exits, returning its exit code.
pub fn wait_process_exit(handle: isize) -> Result<i32, IsolationError> {
    if hcs_fake_mode_enabled() {
        return fake::wait_process_exit(handle);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::wait_process_exit(handle)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Close a process handle obtained from `create_process`.
pub fn close_process_handle(handle: isize) -> Result<(), IsolationError> {
    if hcs_fake_mode_enabled() {
        return fake::close_process_handle(handle);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::close_process_handle(handle)
    }
    #[cfg(not(feature = "hcs"))]
    {
        Err(IsolationError::HcsError(
            "HCS native backend not compiled in (build with --features hcs)".to_string(),
        ))
    }
}

/// Round-trip a compute system through create then terminate — used by
/// `malt-elevate`'s `ManageHcsContainer` dispatch to validate a config works
/// end to end without leaving the compute system running.
pub fn manage_hcs_container(config: &HcsConfig) -> Result<(), IsolationError> {
    let system = create_compute_system(config)?;
    terminate_compute_system(system.raw_handle())
}

mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
    use std::sync::{Mutex, OnceLock};

    static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);
    static NEXT_PROCESS_ID: AtomicI32 = AtomicI32::new(1000);

    fn compute_registry() -> &'static Mutex<HashMap<String, isize>> {
        static REGISTRY: OnceLock<Mutex<HashMap<String, isize>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn process_registry() -> &'static Mutex<HashMap<isize, i32>> {
        static REGISTRY: OnceLock<Mutex<HashMap<isize, i32>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn next_handle() -> isize {
        NEXT_HANDLE.fetch_add(1, Ordering::Relaxed) as isize
    }

    pub fn create_compute_system(config: &HcsConfig) -> HcsComputeSystem {
        let handle = next_handle();
        compute_registry()
            .lock()
            .expect("compute registry lock poisoned")
            .insert(config.id.clone(), handle);
        HcsComputeSystem {
            handle,
            id: config.id.clone(),
        }
    }

    pub fn open_compute_system(id: &str) -> Result<HcsComputeSystem, IsolationError> {
        let handle = compute_registry()
            .lock()
            .expect("compute registry lock poisoned")
            .get(id)
            .copied()
            .ok_or_else(|| {
                IsolationError::HcsError(format!("unknown fake compute system id `{id}`"))
            })?;
        Ok(HcsComputeSystem {
            handle,
            id: id.to_string(),
        })
    }

    pub fn terminate_compute_system(handle: isize) -> Result<(), IsolationError> {
        let mut registry = compute_registry()
            .lock()
            .expect("compute registry lock poisoned");
        let maybe_id = registry
            .iter()
            .find_map(|(id, value)| (*value == handle).then(|| id.clone()));
        match maybe_id {
            Some(id) => {
                registry.remove(&id);
                Ok(())
            }
            None => Err(IsolationError::HcsError(format!(
                "unknown fake compute system handle `{handle}`"
            ))),
        }
    }

    pub fn create_process(
        compute_system: isize,
        _params: &HcsProcessParameters,
    ) -> Result<HcsProcessLaunch, IsolationError> {
        let has_compute = compute_registry()
            .lock()
            .expect("compute registry lock poisoned")
            .values()
            .any(|value| *value == compute_system);
        if !has_compute {
            return Err(IsolationError::HcsError(format!(
                "unknown fake compute handle `{compute_system}`"
            )));
        }

        let handle = next_handle();
        let process_id = NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed) as u32;
        process_registry()
            .lock()
            .expect("process registry lock poisoned")
            .insert(handle, 0);

        Ok(HcsProcessLaunch {
            process: HcsProcess { handle, process_id },
            stdin_handle: None,
            stdout_handle: None,
            stderr_handle: None,
        })
    }

    pub fn wait_process_exit(handle: isize) -> Result<i32, IsolationError> {
        process_registry()
            .lock()
            .expect("process registry lock poisoned")
            .get(&handle)
            .copied()
            .ok_or_else(|| {
                IsolationError::HcsError(format!("unknown fake process handle `{handle}`"))
            })
    }

    pub fn close_process_handle(handle: isize) -> Result<(), IsolationError> {
        let removed = process_registry()
            .lock()
            .expect("process registry lock poisoned")
            .remove(&handle);
        if removed.is_some() {
            Ok(())
        } else {
            Err(IsolationError::HcsError(format!(
                "unknown fake process handle `{handle}`"
            )))
        }
    }
}

#[cfg(feature = "hcs")]
mod native {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::core::PWSTR;
    use windows_sys::Win32::System::HostComputeSystem::{
        HcsCloseComputeSystem, HcsCloseOperation, HcsCloseProcess, HcsCreateComputeSystem,
        HcsCreateOperation, HcsCreateProcess, HcsGetOperationResult,
        HcsGetOperationResultAndProcessInfo, HcsOpenComputeSystem, HcsStartComputeSystem,
        HcsTerminateComputeSystem, HcsWaitForProcessExit, HCS_OPERATION, HCS_PROCESS,
        HCS_PROCESS_INFORMATION, HCS_SYSTEM,
    };

    fn to_wide(value: &str) -> Vec<u16> {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    pub fn create_compute_system(config: &HcsConfig) -> Result<HcsComputeSystem, IsolationError> {
        let id_wide = to_wide(&config.id);
        let cfg_wide = to_wide(&config.config_json);

        // SAFETY: HcsCreateOperation with null context/callback is the
        // documented pattern for synchronous-style HCS calls that poll the
        // operation result immediately below instead of via callback.
        let operation = unsafe { HcsCreateOperation(std::ptr::null(), None) };
        if operation.is_null() {
            return Err(IsolationError::HcsError(
                "HcsCreateOperation returned null".to_string(),
            ));
        }

        let mut handle: HCS_SYSTEM = std::ptr::null_mut();
        // SAFETY: id_wide/cfg_wide are valid null-terminated UTF-16 buffers
        // that outlive this call. `operation` was just checked non-null.
        // `handle` is an out-parameter written by the API on success.
        let hr = unsafe {
            HcsCreateComputeSystem(
                id_wide.as_ptr(),
                cfg_wide.as_ptr(),
                operation,
                std::ptr::null(),
                &mut handle,
            )
        };
        if hr != 0 {
            let details = operation_result_string(operation);
            // SAFETY: operation is a valid handle owned by this function; closing
            // it once, after we're done reading its result, is the documented
            // cleanup call.
            unsafe { HcsCloseOperation(operation) };
            return Err(IsolationError::HcsError(format!(
                "HcsCreateComputeSystem HRESULT={hr:#010x}{details}"
            )));
        }
        // SAFETY: same as above — operation is valid and we're done with it.
        unsafe { HcsCloseOperation(operation) };

        // SAFETY: `handle` was just populated by a successful HcsCreateComputeSystem
        // call above. Passing a null operation/options here matches the
        // synchronous-start pattern used throughout this module.
        let hr = unsafe {
            HcsStartComputeSystem(
                handle,
                std::ptr::null_mut() as HCS_OPERATION,
                std::ptr::null(),
            )
        };
        if hr != 0 {
            let _ = terminate_compute_system(handle as isize);
            return Err(IsolationError::HcsError(format!(
                "HcsStartComputeSystem HRESULT={hr:#010x}"
            )));
        }

        Ok(HcsComputeSystem {
            handle: handle as isize,
            id: config.id.clone(),
        })
    }

    pub fn open_compute_system(id: &str) -> Result<HcsComputeSystem, IsolationError> {
        let id_wide = to_wide(id);
        let mut handle: HCS_SYSTEM = std::ptr::null_mut();
        // SAFETY: id_wide is a valid null-terminated UTF-16 buffer; handle is
        // an out-parameter. 0x001F0000 requests all access rights, matching
        // the access level `create_compute_system` implicitly holds.
        let hr = unsafe { HcsOpenComputeSystem(id_wide.as_ptr(), 0x001F0000, &mut handle) };
        if hr != 0 {
            return Err(IsolationError::HcsError(format!(
                "HcsOpenComputeSystem HRESULT={hr:#010x}"
            )));
        }
        Ok(HcsComputeSystem {
            handle: handle as isize,
            id: id.to_string(),
        })
    }

    pub fn terminate_compute_system(handle: isize) -> Result<(), IsolationError> {
        // SAFETY: caller-provided handle is expected to be a valid HCS_SYSTEM
        // obtained from create/open above; 0x80070057 (E_INVALIDARG) is
        // tolerated as "already terminated" rather than treated as an error.
        let hr = unsafe {
            HcsTerminateComputeSystem(
                handle as HCS_SYSTEM,
                std::ptr::null_mut() as HCS_OPERATION,
                std::ptr::null(),
            )
        };
        if hr != 0 && hr != 0x80070057u32 as i32 {
            return Err(IsolationError::HcsError(format!(
                "HcsTerminateComputeSystem HRESULT={hr:#010x}"
            )));
        }
        // SAFETY: handle is valid per the caller contract above; closing it
        // after termination is the documented HCS cleanup sequence.
        unsafe {
            HcsCloseComputeSystem(handle as HCS_SYSTEM);
        }
        Ok(())
    }

    pub fn create_process(
        compute_system: isize,
        params: &HcsProcessParameters,
    ) -> Result<HcsProcessLaunch, IsolationError> {
        let params_json = serde_json::json!({
            "ApplicationName": params.application_name,
            "CommandLine": params.command_line,
            "WorkingDirectory": params.working_directory,
            "Environment": params.environment.iter().cloned().collect::<BTreeMap<_, _>>(),
            "CreateStdInPipe": false,
            "CreateStdOutPipe": false,
            "CreateStdErrPipe": false
        });
        let params_wide = to_wide(&params_json.to_string());
        // SAFETY: same synchronous-operation pattern as create_compute_system.
        let operation = unsafe { HcsCreateOperation(std::ptr::null(), None) };
        if operation.is_null() {
            return Err(IsolationError::HcsError(
                "HcsCreateOperation returned null".to_string(),
            ));
        }

        let mut process_handle: HCS_PROCESS = std::ptr::null_mut();
        // SAFETY: compute_system is expected to be a valid HCS_SYSTEM from
        // create/open above; params_wide is a valid null-terminated buffer;
        // process_handle is an out-parameter.
        let hr = unsafe {
            HcsCreateProcess(
                compute_system as HCS_SYSTEM,
                params_wide.as_ptr(),
                operation,
                std::ptr::null(),
                &mut process_handle,
            )
        };
        if hr != 0 {
            let details = operation_result_string(operation);
            // SAFETY: operation is valid; this is the standard cleanup call.
            unsafe { HcsCloseOperation(operation) };
            return Err(IsolationError::HcsError(format!(
                "HcsCreateProcess HRESULT={hr:#010x}{details}"
            )));
        }

        let mut process_info: HCS_PROCESS_INFORMATION = unsafe {
            // SAFETY: HCS_PROCESS_INFORMATION is a plain-old-data FFI struct;
            // zero-initializing it before the API populates it below is the
            // documented usage pattern.
            std::mem::zeroed()
        };
        let mut result: PWSTR = std::ptr::null_mut();
        // SAFETY: operation is valid (checked above); process_info and result
        // are out-parameters populated by the API on success.
        let wait_hr = unsafe {
            HcsGetOperationResultAndProcessInfo(operation, &mut process_info, &mut result)
        };
        if wait_hr != 0 {
            let details = operation_result_string(operation);
            // SAFETY: operation and process_handle (if non-null) are valid
            // handles being released on the error path.
            unsafe {
                HcsCloseOperation(operation);
                if !process_handle.is_null() {
                    HcsCloseProcess(process_handle);
                }
            }
            return Err(IsolationError::HcsError(format!(
                "HcsGetOperationResultAndProcessInfo HRESULT={wait_hr:#010x}{details}"
            )));
        }
        // SAFETY: operation is valid; standard cleanup after reading its result.
        unsafe { HcsCloseOperation(operation) };

        Ok(HcsProcessLaunch {
            process: HcsProcess {
                handle: process_handle as isize,
                process_id: process_info.ProcessId,
            },
            stdin_handle: (!process_info.StdInput.is_null())
                .then_some(process_info.StdInput as isize),
            stdout_handle: (!process_info.StdOutput.is_null())
                .then_some(process_info.StdOutput as isize),
            stderr_handle: (!process_info.StdError.is_null())
                .then_some(process_info.StdError as isize),
        })
    }

    pub fn wait_process_exit(handle: isize) -> Result<i32, IsolationError> {
        let mut result: PWSTR = std::ptr::null_mut();
        // SAFETY: handle is expected to be a valid HCS_PROCESS from
        // create_process above; u32::MAX requests an indefinite wait;
        // result is an out-parameter.
        let hr = unsafe { HcsWaitForProcessExit(handle as HCS_PROCESS, u32::MAX, &mut result) };
        if hr != 0 {
            return Err(IsolationError::HcsError(format!(
                "HcsWaitForProcessExit HRESULT={hr:#010x}"
            )));
        }
        if result.is_null() {
            return Ok(0);
        }
        let json = widestr_to_string(result);
        let exit_code = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v.get("ExitCode").and_then(|x| x.as_i64()))
            .unwrap_or(0);
        Ok(exit_code as i32)
    }

    pub fn close_process_handle(handle: isize) -> Result<(), IsolationError> {
        // SAFETY: handle is expected to be a valid HCS_PROCESS obtained from
        // create_process above; closing it once is the documented cleanup.
        unsafe { HcsCloseProcess(handle as HCS_PROCESS) };
        Ok(())
    }

    fn widestr_to_string(ptr: PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        // SAFETY: ptr is a non-null pointer to a null-terminated UTF-16
        // string returned by the HCS API; we scan for the terminator before
        // constructing a slice of exactly that length.
        unsafe {
            while *ptr.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
        }
    }

    fn operation_result_string(operation: HCS_OPERATION) -> String {
        let mut result: PWSTR = std::ptr::null_mut();
        // SAFETY: operation is a valid handle owned by the caller of this
        // helper; result is an out-parameter.
        let hr = unsafe { HcsGetOperationResult(operation, &mut result) };
        if hr != 0 || result.is_null() {
            return String::new();
        }
        let value = widestr_to_string(result);
        if value.is_empty() {
            String::new()
        } else {
            format!(" result={value}")
        }
    }
}

#[cfg(feature = "hcs")]
mod raw {
    use std::ffi::{c_void, CString};
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    pub struct HcsDll {
        module: HMODULE,
    }

    impl HcsDll {
        pub fn load() -> Result<Self, String> {
            let wide: Vec<u16> = std::ffi::OsStr::new("computecore.dll")
                .encode_wide()
                .chain(Some(0))
                .collect();
            // SAFETY: wide is a valid null-terminated UTF-16 buffer for the
            // duration of this call. LoadLibraryW returns null on failure,
            // checked immediately below.
            let module = unsafe { LoadLibraryW(wide.as_ptr()) };
            if module.is_null() {
                // SAFETY: GetLastError reads thread-local state set by the
                // failed call above; always safe to call.
                let error = unsafe { GetLastError() };
                return Err(format!(
                    "failed to load computecore.dll (GetLastError={error})"
                ));
            }
            Ok(Self { module })
        }

        pub fn probe_required_symbols(&self) -> Result<(), String> {
            for symbol in [
                "HcsCreateComputeSystem",
                "HcsOpenComputeSystem",
                "HcsTerminateComputeSystem",
                "HcsCreateProcess",
                "HcsGetOperationResultAndProcessInfo",
            ] {
                self.get_symbol(symbol)?;
            }
            Ok(())
        }

        fn get_symbol(&self, name: &str) -> Result<*const c_void, String> {
            let c_name =
                CString::new(name).map_err(|_| format!("invalid symbol name bytes: {name}"))?;
            // SAFETY: self.module is a valid handle (checked non-null at
            // load time, freed only in Drop); c_name is a valid
            // null-terminated C string for the duration of this call.
            let ptr = unsafe { GetProcAddress(self.module, c_name.as_ptr().cast()) };
            let Some(func) = ptr else {
                // SAFETY: GetLastError reads thread-local state; always safe.
                let error = unsafe { GetLastError() };
                return Err(format!(
                    "missing HCS symbol `{name}` in computecore.dll (GetLastError={error})"
                ));
            };
            Ok(func as *const c_void)
        }
    }

    impl Drop for HcsDll {
        fn drop(&mut self) {
            if !self.module.is_null() {
                // SAFETY: self.module was successfully loaded in `load()` and
                // is only ever freed here, once, on drop.
                unsafe {
                    FreeLibrary(self.module);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn create_compute_system_rejects_empty_id() {
        let config = HcsConfig {
            id: "   ".to_string(),
            config_json: "{}".to_string(),
        };
        let error = create_compute_system(&config).expect_err("must fail");
        assert!(matches!(error, IsolationError::HcsError(_)));
    }

    #[test]
    fn create_compute_system_rejects_invalid_json_config() {
        let config = HcsConfig {
            id: "cs-invalid-json".to_string(),
            config_json: "{not-json".to_string(),
        };
        let error = create_compute_system(&config).expect_err("must fail");
        match error {
            IsolationError::HcsError(message) => assert!(message.contains("valid JSON")),
            other => panic!("expected hcs error, got {other:?}"),
        }
    }

    #[test]
    fn create_process_rejects_empty_command_line() {
        let params = HcsProcessParameters {
            command_line: " ".to_string(),
            ..HcsProcessParameters::default()
        };
        let error = create_process(0, &params).expect_err("must fail");
        assert!(matches!(error, IsolationError::HcsError(_)));
    }

    #[test]
    fn runtime_probe_requires_hcs_feature() {
        let _guard = env_lock();
        // SAFETY: test serializes env mutation via `env_lock`.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE");
        }
        if !cfg!(feature = "hcs") {
            let error = ensure_hcs_runtime().expect_err("must fail without feature");
            assert!(matches!(error, IsolationError::HcsError(_)));
        }
    }

    #[test]
    fn fake_mode_parser_accepts_truthy_values() {
        assert!(hcs_fake_mode_from_env(Some("1")));
        assert!(hcs_fake_mode_from_env(Some("true")));
        assert!(hcs_fake_mode_from_env(Some("YES")));
        assert!(!hcs_fake_mode_from_env(Some("0")));
        assert!(!hcs_fake_mode_from_env(None));
    }

    #[test]
    fn manage_hcs_container_fake_mode_creates_then_cleans_up() {
        let _guard = env_lock();
        // SAFETY: test serializes env mutation via `env_lock`.
        unsafe {
            std::env::set_var("MALT_HCS_FAKE", "1");
        }

        let config = HcsConfig {
            id: "cs-manage-fake".to_string(),
            config_json: "{}".to_string(),
        };

        manage_hcs_container(&config).expect("fake manage_hcs_container should succeed");

        let open_err = open_compute_system(&config.id).expect_err("compute system must be cleaned");
        assert!(matches!(open_err, IsolationError::HcsError(_)));

        // SAFETY: test serializes env mutation via `env_lock`.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE");
        }
    }
}
