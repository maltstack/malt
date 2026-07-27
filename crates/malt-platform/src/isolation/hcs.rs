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
    /// Request console semantics for terminal commands while retaining the
    /// explicit standard streams returned by HCS.
    pub emulate_console: bool,
    /// Ask HCS for the daemon-facing write end of the child's standard input.
    pub create_stdin_pipe: bool,
    /// Ask HCS for the daemon-facing read end of the child's standard output.
    pub create_stdout_pipe: bool,
    /// Ask HCS for the daemon-facing read end of the child's standard error.
    pub create_stderr_pipe: bool,
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

impl Drop for HcsProcess {
    fn drop(&mut self) {
        // HCS process handles are not ordinary Win32 process handles. The
        // dedicated HCS close path is required even when the caller retained
        // the process only long enough to duplicate it into the daemon.
        let _ = close_process_handle(self.handle);
    }
}

/// Result of launching a process, including its I/O pipe handles if requested.
#[derive(Debug)]
pub struct HcsProcessLaunch {
    process: Option<HcsProcess>,
    pub stdin_handle: Option<isize>,
    pub stdout_handle: Option<isize>,
    pub stderr_handle: Option<isize>,
}

/// HCS process resources duplicated into the authenticated daemon process.
/// The numeric handle values are meaningful only in that destination process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcsDuplicatedProcessLaunch {
    pub process_id: u32,
    pub process_handle: u64,
    pub stdin_handle: Option<u64>,
    pub stdout_handle: u64,
    pub stderr_handle: u64,
}

impl Drop for HcsProcessLaunch {
    fn drop(&mut self) {
        for handle in [self.stdin_handle, self.stdout_handle, self.stderr_handle]
            .into_iter()
            .flatten()
        {
            let _ = close_stream_handle(handle);
        }
    }
}

impl HcsProcessLaunch {
    /// Duplicate this launch's OS process and standard handles into one daemon.
    /// The privileged helper obtains the target PID from the authenticated
    /// named-pipe peer; it is deliberately not supplied by the request.
    pub fn duplicate_into_process(
        &self,
        target_process_id: u32,
    ) -> Result<HcsDuplicatedProcessLaunch, IsolationError> {
        let stdout_handle = self.stdout_handle.ok_or_else(|| {
            IsolationError::HcsError("HCS launch did not return a stdout pipe".to_string())
        })?;
        let stderr_handle = self.stderr_handle.ok_or_else(|| {
            IsolationError::HcsError("HCS launch did not return a stderr pipe".to_string())
        })?;
        let process = self.process.as_ref().ok_or_else(|| {
            IsolationError::HcsError(
                "HCS launch process handle was already transferred".to_string(),
            )
        })?;
        let process_handle = if hcs_fake_mode_enabled() {
            process.raw_handle()
        } else {
            open_process_handle(process.process_id)?
        };
        let duplicated = if let Some(stdin_handle) = self.stdin_handle {
            duplicate_handles_into_process(
                target_process_id,
                [process_handle, stdin_handle, stdout_handle, stderr_handle],
            )
            .map(|handles| {
                (
                    handles[0] as u64,
                    Some(handles[1] as u64),
                    handles[2] as u64,
                    handles[3] as u64,
                )
            })
        } else {
            duplicate_handles_into_process(
                target_process_id,
                [process_handle, stdout_handle, stderr_handle],
            )
            .map(|handles| {
                (
                    handles[0] as u64,
                    None,
                    handles[1] as u64,
                    handles[2] as u64,
                )
            })
        };
        if !hcs_fake_mode_enabled() {
            close_process_handle_source(process_handle);
        }
        duplicated.map(
            |(process_handle, stdin_handle, stdout_handle, stderr_handle)| {
                HcsDuplicatedProcessLaunch {
                    process_id: process.process_id,
                    process_handle,
                    stdin_handle,
                    stdout_handle,
                    stderr_handle,
                }
            },
        )
    }

    /// Transfer ownership of the HCS process object to the helper's reaper
    /// after its ordinary process handle and stdio endpoints were handed to the
    /// authenticated daemon. HCS owns stream completion, so this object must
    /// stay live until `HcsWaitForProcessExit` observes the process exit.
    pub fn take_process_for_reaper(&mut self) -> Result<HcsProcess, IsolationError> {
        self.process.take().ok_or_else(|| {
            IsolationError::HcsError(
                "HCS launch process handle was already transferred".to_string(),
            )
        })
    }
}

#[cfg(windows)]
fn open_process_handle(process_id: u32) -> Result<isize, IsolationError> {
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: HCS returned process_id for the process this helper just created.
    // The resulting ordinary process handle is used only for wait/query exit
    // operations after transfer to the authenticated daemon.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            process_id,
        )
    };
    if handle.is_null() {
        return Err(IsolationError::IoError(std::io::Error::last_os_error()));
    }
    Ok(handle as isize)
}

#[cfg(windows)]
fn close_process_handle_source(handle: isize) {
    // SAFETY: open_process_handle returned this ordinary Win32 process handle
    // solely as a source for DuplicateHandle.
    unsafe { windows_sys::Win32::Foundation::CloseHandle(handle as _) };
}

#[cfg(not(windows))]
fn open_process_handle(_process_id: u32) -> Result<isize, IsolationError> {
    Err(IsolationError::UnsupportedPlatform(
        "HCS process handle transfer requires Windows".to_string(),
    ))
}

#[cfg(not(windows))]
fn close_process_handle_source(_handle: isize) {}

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

#[cfg(windows)]
fn close_stream_handle(handle: isize) -> Result<(), IsolationError> {
    if hcs_fake_mode_enabled() {
        return Ok(());
    }
    // SAFETY: `handle` is a Win32 standard-stream handle returned by HCS for
    // this launch and owned by the caller until it is closed exactly once.
    let closed = unsafe { windows_sys::Win32::Foundation::CloseHandle(handle as _) };
    if closed == 0 {
        return Err(IsolationError::IoError(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn close_stream_handle(_handle: isize) -> Result<(), IsolationError> {
    Ok(())
}

#[cfg(windows)]
fn duplicate_handles_into_process<const N: usize>(
    target_process_id: u32,
    sources: [isize; N],
) -> Result<[isize; N], IsolationError> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, DuplicateHandle, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, HANDLE,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE,
    };

    // Fake mode has no kernel handles to duplicate. Retaining the opaque
    // sentinels lets the helper's request/response contract be tested without
    // claiming that a real cross-process transfer occurred.
    if hcs_fake_mode_enabled() {
        return Ok(sources);
    }

    // SAFETY: The target PID is the authenticated helper pipe peer. The
    // helper opens only that process with the minimal access needed to copy
    // the HCS handles it just created.
    let target = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, target_process_id) };
    if target.is_null() {
        return Err(IsolationError::IoError(std::io::Error::last_os_error()));
    }
    // SAFETY: GetCurrentProcess returns this helper's always-valid pseudo-handle.
    let current = unsafe { GetCurrentProcess() };
    let mut duplicated = Vec::<HANDLE>::with_capacity(sources.len());
    for source in sources {
        let mut destination: HANDLE = std::ptr::null_mut();
        // SAFETY: source is a helper-owned live HCS or stream handle and
        // target was opened above with PROCESS_DUP_HANDLE.
        let copied = unsafe {
            DuplicateHandle(
                current,
                source as HANDLE,
                target,
                &mut destination,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if copied == 0 {
            let error = std::io::Error::last_os_error();
            for remote in duplicated {
                let mut local: HANDLE = std::ptr::null_mut();
                // SAFETY: remote is a handle in `target` that this function
                // created. DUPLICATE_CLOSE_SOURCE releases it in that target;
                // any local duplicate returned solely to satisfy the API is
                // immediately closed below.
                let closed = unsafe {
                    DuplicateHandle(
                        target,
                        remote,
                        current,
                        &mut local,
                        0,
                        0,
                        DUPLICATE_CLOSE_SOURCE | DUPLICATE_SAME_ACCESS,
                    )
                };
                if closed != 0 && !local.is_null() {
                    // SAFETY: local is the temporary current-process handle
                    // returned by DuplicateHandle above.
                    unsafe { CloseHandle(local) };
                }
            }
            // SAFETY: target was opened successfully above and is owned here.
            unsafe { CloseHandle(target) };
            return Err(IsolationError::IoError(error));
        }
        duplicated.push(destination);
    }
    // SAFETY: target was opened successfully above and is owned here.
    unsafe { CloseHandle(target) };
    let handles: [HANDLE; N] = duplicated.try_into().map_err(|_| {
        IsolationError::HcsError("internal HCS handle duplication count mismatch".to_string())
    })?;
    Ok(handles.map(|handle| handle as isize))
}

#[cfg(not(windows))]
fn duplicate_handles_into_process<const N: usize>(
    _target_process_id: u32,
    _sources: [isize; N],
) -> Result<[isize; N], IsolationError> {
    Err(IsolationError::UnsupportedPlatform(
        "HCS handle duplication requires Windows".to_string(),
    ))
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
        return fake::create_compute_system(config);
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

/// Check whether a process inside a compute system has exited without blocking.
/// `Ok(None)` means the process is still running.
pub fn try_wait_process_exit(handle: isize) -> Result<Option<i32>, IsolationError> {
    if hcs_fake_mode_enabled() {
        return fake::try_wait_process_exit(handle);
    }
    ensure_hcs_runtime()?;
    #[cfg(feature = "hcs")]
    {
        native::try_wait_process_exit(handle)
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

    pub fn create_compute_system(config: &HcsConfig) -> Result<HcsComputeSystem, IsolationError> {
        let handle = next_handle();
        compute_registry()
            .lock()
            .expect("compute registry lock poisoned")
            .insert(config.id.clone(), handle);
        let system = HcsComputeSystem {
            handle,
            id: config.id.clone(),
        };
        if std::env::var_os("MALT_HCS_FAKE_START_FAIL").is_some() {
            terminate_compute_system(handle)?;
            return Err(IsolationError::HcsError(
                "fake HcsStartComputeSystem failure".to_string(),
            ));
        }
        Ok(system)
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
        params: &HcsProcessParameters,
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
            process: Some(HcsProcess { handle, process_id }),
            stdin_handle: params.create_stdin_pipe.then(next_handle),
            stdout_handle: params.create_stdout_pipe.then(next_handle),
            stderr_handle: params.create_stderr_pipe.then(next_handle),
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

    pub fn try_wait_process_exit(handle: isize) -> Result<Option<i32>, IsolationError> {
        Ok(Some(wait_process_exit(handle)?))
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
    use windows_sys::Win32::Foundation::{
        LocalFree, HCS_E_ACCESS_DENIED, HCS_E_CONNECTION_CLOSED, HCS_E_CONNECTION_TIMEOUT,
        HCS_E_CONNECT_FAILED, HCS_E_GUEST_CRITICAL_ERROR, HCS_E_HYPERV_NOT_INSTALLED,
        HCS_E_IMAGE_MISMATCH, HCS_E_INVALID_JSON, HCS_E_INVALID_LAYER, HCS_E_INVALID_STATE,
        HCS_E_OPERATION_ALREADY_CANCELLED, HCS_E_OPERATION_ALREADY_STARTED,
        HCS_E_OPERATION_NOT_STARTED, HCS_E_OPERATION_PENDING,
        HCS_E_OPERATION_RESULT_ALLOCATION_FAILED, HCS_E_OPERATION_TIMEOUT,
        HCS_E_PROCESS_ALREADY_STOPPED, HCS_E_PROCESS_INFO_NOT_AVAILABLE, HCS_E_PROTOCOL_ERROR,
        HCS_E_SERVICE_DISCONNECT, HCS_E_SERVICE_NOT_AVAILABLE, HCS_E_SYSTEM_ALREADY_EXISTS,
        HCS_E_SYSTEM_ALREADY_STOPPED, HCS_E_SYSTEM_NOT_CONFIGURED_FOR_OPERATION,
        HCS_E_SYSTEM_NOT_FOUND, HCS_E_TERMINATED, HCS_E_TERMINATED_DURING_START,
        HCS_E_UNEXPECTED_EXIT, HCS_E_UNKNOWN_MESSAGE, HCS_E_UNSUPPORTED_PROTOCOL_VERSION,
        HCS_E_WINDOWS_INSIDER_REQUIRED, HLOCAL,
    };
    use windows_sys::Win32::System::HostComputeSystem::{
        HcsCloseComputeSystem, HcsCloseOperation, HcsCloseProcess, HcsCreateComputeSystem,
        HcsCreateOperation, HcsCreateProcess, HcsGetOperationResult, HcsOpenComputeSystem,
        HcsStartComputeSystem, HcsTerminateComputeSystem, HcsWaitForOperationResult,
        HcsWaitForOperationResultAndProcessInfo, HcsWaitForProcessExit, HCS_OPERATION, HCS_PROCESS,
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

        let mut handle: HCS_SYSTEM = std::ptr::null_mut();
        run_operation("HcsCreateComputeSystem", |operation| {
            // SAFETY: id_wide/cfg_wide are valid null-terminated UTF-16
            // buffers that outlive this call; `operation` is non-null per
            // `run_operation`; `handle` is an out-parameter written by the API.
            unsafe {
                HcsCreateComputeSystem(
                    id_wide.as_ptr(),
                    cfg_wide.as_ptr(),
                    operation,
                    std::ptr::null(),
                    &mut handle,
                )
            }
        })?;

        if handle.is_null() {
            return Err(IsolationError::HcsError(
                "HcsCreateComputeSystem reported success but produced no handle".to_string(),
            ));
        }

        // Starting is a *second* asynchronous operation, not a continuation of
        // the first, so it needs its own operation handle. If it fails the
        // compute system already exists and must be torn down before
        // returning, or it leaks until the host is rebooted.
        if let Err(error) = run_operation("HcsStartComputeSystem", |operation| {
            // SAFETY: `handle` is a non-null compute system from the create
            // call above; `operation` is non-null per `run_operation`; a null
            // options string is the documented "no options" form.
            unsafe { HcsStartComputeSystem(handle, operation, std::ptr::null()) }
        }) {
            return match terminate_compute_system(handle as isize) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(IsolationError::HcsError(format!(
                    "{error}; HcsTerminateComputeSystem cleanup also failed: {cleanup_error}"
                ))),
            };
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
                "HcsOpenComputeSystem {}",
                describe_hresult(hr)
            )));
        }
        Ok(HcsComputeSystem {
            handle: handle as isize,
            id: id.to_string(),
        })
    }

    pub fn terminate_compute_system(handle: isize) -> Result<(), IsolationError> {
        // Terminating is asynchronous like every other mutating HCS call, and
        // was the second site passing a null operation handle -- so tearing a
        // container down faulted just as reliably as starting one.
        let outcome = run_operation("HcsTerminateComputeSystem", |operation| {
            // SAFETY: caller-provided handle is expected to be a valid
            // HCS_SYSTEM from create/open above; `operation` is non-null per
            // `run_operation`; null options is the documented no-options form.
            unsafe { HcsTerminateComputeSystem(handle as HCS_SYSTEM, operation, std::ptr::null()) }
        });

        // The handle must be closed whether or not termination succeeded --
        // returning early here would leak it on exactly the paths where
        // something has already gone wrong.
        // SAFETY: handle is valid per the caller contract above; closing it
        // after termination is the documented HCS cleanup sequence.
        unsafe {
            HcsCloseComputeSystem(handle as HCS_SYSTEM);
        }

        match outcome {
            Ok(_) => Ok(()),
            // A system that is already gone is the state the caller asked
            // for, not a failure. HCS reports that as E_INVALIDARG against
            // the stale handle.
            Err(IsolationError::HcsError(message))
                if message.contains(&format!("{:#010x}", 0x80070057u32)) =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
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
            "EmulateConsole": params.emulate_console,
            "CreateStdInPipe": params.create_stdin_pipe,
            "CreateStdOutPipe": params.create_stdout_pipe,
            "CreateStdErrPipe": params.create_stderr_pipe
        });
        let params_wide = to_wide(&params_json.to_string());
        // Not routed through `run_operation`: this call needs the process-info
        // out-parameter, which requires the `...AndProcessInfo` wait variant
        // rather than the plain one. The operation lifecycle below is otherwise
        // identical -- create, call, *wait*, close on every path.
        // SAFETY: documented form for an operation with no completion callback.
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
                "HcsCreateProcess {}{details}",
                describe_hresult(hr)
            )));
        }

        let mut process_info: HCS_PROCESS_INFORMATION = unsafe {
            // SAFETY: HCS_PROCESS_INFORMATION is a plain-old-data FFI struct;
            // zero-initializing it before the API populates it below is the
            // documented usage pattern.
            std::mem::zeroed()
        };
        let mut result: PWSTR = std::ptr::null_mut();
        // `Wait...`, not `Get...`. The non-waiting variant returns whatever
        // state the operation happens to be in, which for a launch that has
        // not completed yet is `HCS_E_OPERATION_PENDING` -- so process
        // creation failed or succeeded depending on timing.
        //
        // SAFETY: operation is valid (checked above); process_info and result
        // are out-parameters populated by the API, and we take ownership of
        // the result document.
        let wait_hr = unsafe {
            HcsWaitForOperationResultAndProcessInfo(
                operation,
                OPERATION_TIMEOUT_MS,
                &mut process_info,
                &mut result,
            )
        };
        let result_document = take_result_document(result);
        if wait_hr != 0 {
            // SAFETY: operation and process_handle (if non-null) are valid
            // handles being released on the error path.
            unsafe {
                HcsCloseOperation(operation);
                if !process_handle.is_null() {
                    HcsCloseProcess(process_handle);
                }
            }
            let detail = if result_document.is_empty() {
                String::new()
            } else {
                format!(" result={result_document}")
            };
            return Err(IsolationError::HcsError(format!(
                "HcsWaitForOperationResultAndProcessInfo {}{detail}",
                describe_hresult(wait_hr)
            )));
        }
        // SAFETY: operation is valid; standard cleanup after reading its result.
        unsafe { HcsCloseOperation(operation) };

        Ok(HcsProcessLaunch {
            process: Some(HcsProcess {
                handle: process_handle as isize,
                process_id: process_info.ProcessId,
            }),
            stdin_handle: (!process_info.StdInput.is_null())
                .then_some(process_info.StdInput as isize),
            stdout_handle: (!process_info.StdOutput.is_null())
                .then_some(process_info.StdOutput as isize),
            stderr_handle: (!process_info.StdError.is_null())
                .then_some(process_info.StdError as isize),
        })
    }

    pub fn wait_process_exit(handle: isize) -> Result<i32, IsolationError> {
        wait_for_process_exit(handle, u32::MAX)?.ok_or_else(|| {
            IsolationError::HcsError(
                "infinite HCS process wait returned without an exit result".to_string(),
            )
        })
    }

    pub fn try_wait_process_exit(handle: isize) -> Result<Option<i32>, IsolationError> {
        wait_for_process_exit(handle, 0)
    }

    fn wait_for_process_exit(
        handle: isize,
        timeout_ms: u32,
    ) -> Result<Option<i32>, IsolationError> {
        let mut result: PWSTR = std::ptr::null_mut();
        // SAFETY: handle is expected to be a valid HCS_PROCESS from
        // create_process above; `timeout_ms` is either zero for polling or
        // u32::MAX for an indefinite wait; result is an out-parameter.
        let hr = unsafe { HcsWaitForProcessExit(handle as HCS_PROCESS, timeout_ms, &mut result) };
        if hr == HCS_E_OPERATION_TIMEOUT {
            return Ok(None);
        }
        if hr != 0 {
            return Err(IsolationError::HcsError(format!(
                "HcsWaitForProcessExit {}",
                describe_hresult(hr)
            )));
        }
        if result.is_null() {
            return Ok(Some(0));
        }
        let json = take_result_document(result);
        let exit_code = serde_json::from_str::<serde_json::Value>(&json)
            .ok()
            .and_then(|v| v.get("ExitCode").and_then(|x| x.as_i64()))
            .unwrap_or(0);
        Ok(Some(exit_code as i32))
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
        let value = take_result_document(result);
        if value.is_empty() {
            String::new()
        } else {
            format!(" result={value}")
        }
    }

    /// Read an HCS result document and release it.
    ///
    /// HCS allocates result documents with `LocalAlloc` and transfers
    /// ownership to the caller, so every out-parameter of this kind has to be
    /// handed to `LocalFree`. Nothing in this module used to do that; each
    /// call leaked the document.
    fn take_result_document(ptr: PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let value = widestr_to_string(ptr);
        // SAFETY: `ptr` is a non-null document allocated by computecore and
        // owned by us per the HCS contract. It is not read after this call.
        unsafe { LocalFree(ptr as HLOCAL) };
        value
    }

    /// Name the well-known HCS failures instead of emitting a bare HRESULT.
    ///
    /// This is not cosmetic. `0x8037011b` on its own is what kept this
    /// module's real problem — the daemon not holding Hyper-V Administrators
    /// rights — unreadable once the crash above it was fixed. A caller who
    /// sees the name can act on it; a caller who sees the hex cannot.
    ///
    /// Matched against the `windows_sys` constants rather than literals on
    /// purpose: a hand-written table of these was wrong in more entries than
    /// it was right when checked against the real values.
    fn hcs_error_name(hr: i32) -> Option<&'static str> {
        // `ACCESS_DENIED` carries the remedy, since it is the one a correctly
        // configured host still hits and the one with a non-obvious fix.
        if hr == HCS_E_ACCESS_DENIED {
            return Some(
                "HCS_E_ACCESS_DENIED -- the caller is not an administrator and not a \
                 member of the Hyper-V Administrators group (see https://aka.ms/hcsadmin)",
            );
        }
        let name = match hr {
            HCS_E_CONNECTION_CLOSED => "HCS_E_CONNECTION_CLOSED",
            HCS_E_CONNECTION_TIMEOUT => "HCS_E_CONNECTION_TIMEOUT",
            HCS_E_CONNECT_FAILED => "HCS_E_CONNECT_FAILED",
            HCS_E_GUEST_CRITICAL_ERROR => "HCS_E_GUEST_CRITICAL_ERROR",
            HCS_E_HYPERV_NOT_INSTALLED => "HCS_E_HYPERV_NOT_INSTALLED",
            HCS_E_IMAGE_MISMATCH => "HCS_E_IMAGE_MISMATCH",
            HCS_E_INVALID_JSON => "HCS_E_INVALID_JSON",
            HCS_E_INVALID_LAYER => "HCS_E_INVALID_LAYER",
            HCS_E_INVALID_STATE => "HCS_E_INVALID_STATE",
            HCS_E_OPERATION_ALREADY_CANCELLED => "HCS_E_OPERATION_ALREADY_CANCELLED",
            HCS_E_OPERATION_ALREADY_STARTED => "HCS_E_OPERATION_ALREADY_STARTED",
            HCS_E_OPERATION_NOT_STARTED => "HCS_E_OPERATION_NOT_STARTED",
            HCS_E_OPERATION_PENDING => "HCS_E_OPERATION_PENDING",
            HCS_E_OPERATION_RESULT_ALLOCATION_FAILED => "HCS_E_OPERATION_RESULT_ALLOCATION_FAILED",
            HCS_E_OPERATION_TIMEOUT => "HCS_E_OPERATION_TIMEOUT",
            HCS_E_PROCESS_ALREADY_STOPPED => "HCS_E_PROCESS_ALREADY_STOPPED",
            HCS_E_PROCESS_INFO_NOT_AVAILABLE => "HCS_E_PROCESS_INFO_NOT_AVAILABLE",
            HCS_E_PROTOCOL_ERROR => "HCS_E_PROTOCOL_ERROR",
            HCS_E_SERVICE_DISCONNECT => "HCS_E_SERVICE_DISCONNECT",
            HCS_E_SERVICE_NOT_AVAILABLE => "HCS_E_SERVICE_NOT_AVAILABLE",
            HCS_E_SYSTEM_ALREADY_EXISTS => "HCS_E_SYSTEM_ALREADY_EXISTS",
            HCS_E_SYSTEM_ALREADY_STOPPED => "HCS_E_SYSTEM_ALREADY_STOPPED",
            HCS_E_SYSTEM_NOT_CONFIGURED_FOR_OPERATION => {
                "HCS_E_SYSTEM_NOT_CONFIGURED_FOR_OPERATION"
            }
            HCS_E_SYSTEM_NOT_FOUND => "HCS_E_SYSTEM_NOT_FOUND",
            HCS_E_TERMINATED => "HCS_E_TERMINATED",
            HCS_E_TERMINATED_DURING_START => "HCS_E_TERMINATED_DURING_START",
            HCS_E_UNEXPECTED_EXIT => "HCS_E_UNEXPECTED_EXIT",
            HCS_E_UNKNOWN_MESSAGE => "HCS_E_UNKNOWN_MESSAGE",
            HCS_E_UNSUPPORTED_PROTOCOL_VERSION => "HCS_E_UNSUPPORTED_PROTOCOL_VERSION",
            HCS_E_WINDOWS_INSIDER_REQUIRED => "HCS_E_WINDOWS_INSIDER_REQUIRED",
            _ => return None,
        };
        Some(name)
    }

    fn describe_hresult(hr: i32) -> String {
        match hcs_error_name(hr) {
            Some(name) => format!("HRESULT={hr:#010x} ({name})"),
            None => format!("HRESULT={hr:#010x}"),
        }
    }

    /// How long to wait for a single HCS operation to complete.
    ///
    /// Bounded rather than infinite on purpose: these calls run on a session's
    /// thread, and an HCS operation that never completes would otherwise wedge
    /// that session permanently with no way to observe why.
    const OPERATION_TIMEOUT_MS: u32 = 60_000;

    /// Run one asynchronous HCS call to completion.
    ///
    /// **Every mutating HCS API is asynchronous.** It takes an
    /// `HCS_OPERATION`, returns once the request is *queued*, and reports the
    /// real outcome through that operation. Two consequences, and getting
    /// either wrong is a process fault rather than an error return:
    ///
    /// 1. **The operation handle must be valid.** `HcsStartComputeSystem` and
    ///    `HcsTerminateComputeSystem` were previously passed a null one, with
    ///    a comment claiming a "synchronous-start pattern". No such pattern
    ///    exists — computecore dereferences the handle unconditionally, so the
    ///    process died with `STATUS_ACCESS_VIOLATION` before any HRESULT could
    ///    come back. That is the whole of docs/briefs/006.
    /// 2. **The result must be waited for.** `S_OK` from the call itself means
    ///    "accepted", not "done". `HcsCreateComputeSystem` returned `S_OK` and
    ///    a non-null handle on a host that cannot actually run containers.
    ///
    /// The operation is closed on every path, including the error paths.
    fn run_operation<F>(name: &str, call: F) -> Result<String, IsolationError>
    where
        F: FnOnce(HCS_OPERATION) -> i32,
    {
        // SAFETY: the documented way to create an operation with no completion
        // callback. A null context is only read back by a callback, and there
        // is none.
        let operation = unsafe { HcsCreateOperation(std::ptr::null(), None) };
        if operation.is_null() {
            return Err(IsolationError::HcsError(format!(
                "{name}: HcsCreateOperation returned null"
            )));
        }

        let hr = call(operation);
        if hr != 0 {
            let details = operation_result_string(operation);
            // SAFETY: `operation` is non-null (checked above) and is closed
            // exactly once on this path.
            unsafe { HcsCloseOperation(operation) };
            return Err(IsolationError::HcsError(format!(
                "{name} {}{details}",
                describe_hresult(hr)
            )));
        }

        let mut result: PWSTR = std::ptr::null_mut();
        // SAFETY: `operation` is valid and was accepted by the call above;
        // `result` is an out-parameter whose document we take ownership of.
        let wait_hr =
            unsafe { HcsWaitForOperationResult(operation, OPERATION_TIMEOUT_MS, &mut result) };
        let document = take_result_document(result);
        // SAFETY: `operation` is valid; closed exactly once, after its result
        // has been read.
        unsafe { HcsCloseOperation(operation) };

        if wait_hr != 0 {
            let detail = if document.is_empty() {
                String::new()
            } else {
                format!(" result={document}")
            };
            return Err(IsolationError::HcsError(format!(
                "{name} failed asynchronously: {}{detail}",
                describe_hresult(wait_hr)
            )));
        }
        Ok(document)
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

    #[test]
    fn fake_process_launch_returns_every_requested_standard_pipe() {
        let _guard = env_lock();
        // SAFETY: test serializes env mutation via `env_lock`.
        unsafe {
            std::env::set_var("MALT_HCS_FAKE", "1");
        }
        let system = create_compute_system(&HcsConfig {
            id: "cs-process-pipes".to_string(),
            config_json: "{}".to_string(),
        })
        .expect("create fake compute system");
        let launch = create_process(
            system.raw_handle(),
            &HcsProcessParameters {
                command_line: "cmd.exe /c exit 0".to_string(),
                create_stdin_pipe: true,
                create_stdout_pipe: true,
                create_stderr_pipe: true,
                ..HcsProcessParameters::default()
            },
        )
        .expect("create fake HCS process");
        assert!(launch.stdin_handle.is_some());
        assert!(launch.stdout_handle.is_some());
        assert!(launch.stderr_handle.is_some());
        // Drop while fake mode remains enabled: `HcsProcess` owns the process
        // handle and closes it through the matching HCS backend.
        drop(launch);
        terminate_compute_system(system.raw_handle()).expect("terminate fake compute system");
        // SAFETY: paired with the serialized test-only mutation above.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE");
        }
    }

    #[test]
    fn fake_start_failure_removes_the_created_compute_system() {
        let _guard = env_lock();
        // SAFETY: test serializes environment mutation via env_lock.
        unsafe {
            std::env::set_var("MALT_HCS_FAKE", "1");
            std::env::set_var("MALT_HCS_FAKE_START_FAIL", "1");
        }
        let config = HcsConfig {
            id: "cs-start-failure-cleanup".to_string(),
            config_json: "{}".to_string(),
        };
        let error = create_compute_system(&config).expect_err("fake start must fail");
        assert!(matches!(error, IsolationError::HcsError(_)));
        assert!(
            open_compute_system(&config.id).is_err(),
            "a failed start must not leave a compute system behind"
        );
        // SAFETY: paired with the serialized test-only mutations above.
        unsafe {
            std::env::remove_var("MALT_HCS_FAKE_START_FAIL");
            std::env::remove_var("MALT_HCS_FAKE");
        }
    }
}
