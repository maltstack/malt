use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::ipc::NamedPipeClient;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_DIR_NOT_EMPTY, ERROR_FAILED_SERVICE_CONTROLLER_CONNECT,
    ERROR_SERVICE_MARKED_FOR_DELETE, ERROR_SERVICE_NOT_ACTIVE, ERROR_SERVICE_SPECIFIC_ERROR,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
    OpenServiceW, QueryServiceStatus, RegisterServiceCtrlHandlerExW, SetServiceStatus,
    StartServiceCtrlDispatcherW, StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT,
    SC_MANAGER_CREATE_SERVICE, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_STOP, SERVICE_DEMAND_START,
    SERVICE_ERROR_NORMAL, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOP, SERVICE_STOPPED,
    SERVICE_STOP_PENDING, SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_ProgramFiles, SHGetKnownFolderPath, ShellExecuteExW, KF_FLAG_DEFAULT,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

type ServiceWork = Box<dyn FnOnce(&StopSignal) -> io::Result<()> + Send + 'static>;
const DELETE_ACCESS: u32 = 0x0001_0000;

static SERVICE_CONTEXT: OnceLock<Arc<ServiceContext>> = OnceLock::new();

/// Resolve the machine's Program Files directory without trusting inherited
/// environment variables in an elevated child process.
pub fn program_files_path() -> io::Result<PathBuf> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: `raw` points to writable PWSTR storage, the known-folder GUID is
    // static, and a null token requests the current machine/user context. The
    // returned allocation is released with CoTaskMemFree below.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_ProgramFiles,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut raw,
        )
    };
    if result < 0 {
        return Err(io::Error::other(format!(
            "SHGetKnownFolderPath(FOLDERID_ProgramFiles) failed: HRESULT=0x{:08X}",
            result as u32
        )));
    }
    if raw.is_null() {
        return Err(io::Error::other(
            "SHGetKnownFolderPath returned a null Program Files path",
        ));
    }
    let mut length = 0usize;
    // SAFETY: a successful SHGetKnownFolderPath call returns a NUL-terminated
    // UTF-16 allocation. This loop reads only through that terminator.
    while unsafe { *raw.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `raw` contains `length` initialized UTF-16 code units before its
    // terminator, and OsString copies them before the allocation is released.
    let path = std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, length) });
    // SAFETY: `raw` was allocated by SHGetKnownFolderPath and has not been
    // freed or transferred.
    unsafe { CoTaskMemFree(raw.cast()) };
    Ok(PathBuf::from(path))
}

/// Copy a service executable into an administrator-owned destination and
/// publish it atomically within that directory.
///
/// The caller chooses the destination policy. Production callers should use
/// [`program_files_path`] rather than an inherited environment variable.
pub fn deploy_service_executable(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("service executable source is absent: {}", source.display()),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "service executable destination has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    let file_name = destination.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "service executable destination has no file name",
        )
    })?;
    let staging = parent.join(format!(
        ".{}.{}.installing",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    match std::fs::remove_file(&staging) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let deployment = (|| {
        let mut source_file = std::fs::File::open(source)?;
        let expected = source_file.metadata()?.len();
        let mut staging_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        let copied = io::copy(&mut source_file, &mut staging_file)?;
        if copied != expected {
            return Err(io::Error::other(format!(
                "service executable copy was incomplete: copied {copied} of {expected} bytes"
            )));
        }
        staging_file.sync_all()?;
        drop(staging_file);
        atomic_replace(&staging, destination)
    })();
    if deployment.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    deployment
}

/// Remove a deployed service executable and its immediate directory when that
/// directory is empty. Missing files and directories are already clean.
pub fn remove_service_executable(destination: &Path) -> io::Result<()> {
    match std::fs::remove_file(destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = destination.parent() {
        match std::fs::remove_dir(parent) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(ERROR_DIR_NOT_EMPTY as i32) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let source = path_wide(source)?;
    let destination = path_wide(destination)?;
    // SAFETY: both path buffers are NUL terminated and live through the call.
    // The source and destination share a directory, and the flags request
    // replacement plus durable completion before returning.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn path_wide(path: &Path) -> io::Result<Vec<u16>> {
    if path.as_os_str().encode_wide().any(|unit| unit == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "service executable path contains an embedded NUL",
        ));
    }
    Ok(path.as_os_str().encode_wide().chain(Some(0)).collect())
}

/// Cooperative stop signal delivered to a Windows service workload.
#[derive(Debug, Clone)]
pub struct StopSignal {
    requested: Arc<AtomicBool>,
}

impl StopSignal {
    /// Returns true after the Service Control Manager has requested a stop.
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

/// Return whether the current process owns an elevated Windows access token.
pub fn is_current_process_elevated() -> io::Result<bool> {
    let mut token = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle, `token` points
    // to writable storage, and TOKEN_QUERY cannot modify the token.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY: `token` is valid and `elevation` is writable storage sized for
    // TOKEN_ELEVATION, which is the requested token-information class.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    let query_error = (queried == 0).then(io::Error::last_os_error);
    // SAFETY: token is owned from the successful OpenProcessToken call.
    if unsafe { CloseHandle(token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if let Some(error) = query_error {
        return Err(error);
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// Request UAC consent for an explicit child command and wait for its exit.
///
/// A user declining the consent prompt returns the Windows cancellation error;
/// no installation or removal command has run in that case.
pub fn run_elevated(executable: &Path, arguments: &[&str]) -> io::Result<u32> {
    let executable = executable.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "elevated executable path is not valid UTF-8",
        )
    })?;
    let executable = wide(executable)?;
    let verb = wide("runas")?;
    let arguments = wide(&command_arguments(arguments)?)?;
    let mut execute = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: arguments.as_ptr(),
        lpDirectory: std::ptr::null(),
        nShow: SW_SHOWNORMAL,
        hInstApp: std::ptr::null_mut(),
        lpIDList: std::ptr::null_mut(),
        lpClass: std::ptr::null(),
        hkeyClass: std::ptr::null_mut(),
        dwHotKey: 0,
        Anonymous: Default::default(),
        hProcess: std::ptr::null_mut(),
    };
    // SAFETY: all wide-string buffers are NUL terminated and live through the
    // call; the structure is fully initialized and requests a process handle.
    if unsafe { ShellExecuteExW(&mut execute) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if execute.hProcess.is_null() {
        return Err(io::Error::other(
            "elevated process did not return a waitable process handle",
        ));
    }
    // SAFETY: hProcess is returned by ShellExecuteExW and owned here.
    let waited = unsafe { WaitForSingleObject(execute.hProcess, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        let wait_error = (waited == WAIT_FAILED).then(io::Error::last_os_error);
        // SAFETY: hProcess is still owned here after the wait failure.
        let close = unsafe { CloseHandle(execute.hProcess) };
        if close == 0 {
            return Err(io::Error::last_os_error());
        }
        return Err(wait_error.unwrap_or_else(|| {
            io::Error::other("waiting for elevated process returned an unexpected status")
        }));
    }
    let mut exit_code = 0u32;
    // SAFETY: hProcess remains valid after a successful wait and exit_code is
    // writable storage for the documented process exit code.
    let received = unsafe { GetExitCodeProcess(execute.hProcess, &mut exit_code) };
    let exit_error = (received == 0).then(io::Error::last_os_error);
    // SAFETY: hProcess is owned here and must be closed exactly once.
    if unsafe { CloseHandle(execute.hProcess) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if let Some(error) = exit_error {
        return Err(error);
    }
    Ok(exit_code)
}

/// Run a workload under the Windows Service Control Manager.
///
/// `wake_pipe` is connected after a stop control so a synchronous named-pipe
/// accept can observe the stop request rather than leaving the service stuck.
/// This function is only meaningful for a process launched by the SCM.
pub fn run_service(
    name: &str,
    wake_pipe: Option<&str>,
    work: impl FnOnce(&StopSignal) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let name = wide(name)?;
    let context = Arc::new(ServiceContext {
        stop: StopSignal {
            requested: Arc::new(AtomicBool::new(false)),
        },
        wake_pipe: wake_pipe.map(str::to_owned),
        status: Mutex::new(None),
        work: Mutex::new(Some(Box::new(work))),
        error: Mutex::new(None),
    });
    SERVICE_CONTEXT.set(context.clone()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Windows service host can be initialized only once per process",
        )
    })?;
    let entries = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_ptr().cast_mut(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    // SAFETY: `entries` is a NUL-terminated service table and remains alive
    // until StartServiceCtrlDispatcherW returns; its callback has the required
    // system ABI and reads its state from the process-lifetime context above.
    if unsafe { StartServiceCtrlDispatcherW(entries.as_ptr()) } == 0 {
        // SAFETY: GetLastError reads the last error set by the preceding call.
        let error = unsafe { GetLastError() };
        let detail = if error == ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
            "malt-elevate --service must be launched by the Windows Service Control Manager"
        } else {
            "StartServiceCtrlDispatcherW failed"
        };
        return Err(io::Error::other(format!("{detail}: {error}")));
    }
    let mut error = context
        .error
        .lock()
        .map_err(|_| io::Error::other("service error state lock poisoned"))?;
    match error.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct ServiceContext {
    stop: StopSignal,
    wake_pipe: Option<String>,
    status: Mutex<Option<usize>>,
    work: Mutex<Option<ServiceWork>>,
    error: Mutex<Option<io::Error>>,
}

unsafe extern "system" fn service_main(_count: u32, arguments: *mut *mut u16) {
    let Some(context) = SERVICE_CONTEXT.get() else {
        return;
    };
    if arguments.is_null() || unsafe { (*arguments).is_null() } {
        context.record_error(io::Error::new(
            io::ErrorKind::InvalidData,
            "Service Control Manager supplied no service name",
        ));
        return;
    }
    // SAFETY: the Service Control Manager invokes this callback with a valid
    // first argument pointing to the registered NUL-terminated service name.
    let name = unsafe { *arguments };
    // SAFETY: `name` remains valid for this callback; `service_control` has
    // the documented handler ABI and reads only the process-lifetime context.
    let status =
        unsafe { RegisterServiceCtrlHandlerExW(name, Some(service_control), std::ptr::null()) };
    if status.is_null() {
        context.record_error(io::Error::last_os_error());
        return;
    }
    if let Err(error) = context.set_status(status, SERVICE_START_PENDING, 0) {
        context.record_error(error);
        return;
    }
    if let Ok(mut registered) = context.status.lock() {
        *registered = Some(status as usize);
    } else {
        context.record_error(io::Error::other("service status lock poisoned"));
        return;
    }
    if let Err(error) = context.set_status(status, SERVICE_RUNNING, SERVICE_ACCEPT_STOP) {
        context.record_error(error);
        return;
    }
    let work = match context.work.lock() {
        Ok(mut work) => work.take(),
        Err(_) => {
            context.record_error(io::Error::other("service work lock poisoned"));
            None
        }
    };
    match work {
        Some(work) => {
            if let Err(error) = work(&context.stop) {
                context.record_error(error);
            }
        }
        None => context.record_error(io::Error::other("service work was already consumed")),
    }
    let failed = context.error.lock().map_or(true, |error| error.is_some());
    let _ = context.set_status_with_exit(
        status,
        SERVICE_STOPPED,
        0,
        if failed {
            ERROR_SERVICE_SPECIFIC_ERROR
        } else {
            0
        },
    );
}

unsafe extern "system" fn service_control(
    control: u32,
    _event_type: u32,
    _event_data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    let Some(context) = SERVICE_CONTEXT.get() else {
        return 0;
    };
    if control == SERVICE_CONTROL_STOP {
        context.stop.requested.store(true, Ordering::Release);
        if let Ok(status) = context.status.lock() {
            if let Some(status) = *status {
                if let Err(error) =
                    context.set_status(status as SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, 0)
                {
                    context.record_error(error);
                }
            }
        } else {
            context.record_error(io::Error::other("service status lock poisoned"));
        }
        if let Some(pipe) = &context.wake_pipe {
            if let Err(error) = NamedPipeClient::connect(pipe) {
                context.record_error(io::Error::new(
                    error.kind(),
                    format!("wake named-pipe accept after service stop: {error}"),
                ));
            }
        }
    }
    0
}

impl ServiceContext {
    fn set_status(
        &self,
        handle: SERVICE_STATUS_HANDLE,
        state: u32,
        accepted_controls: u32,
    ) -> io::Result<()> {
        self.set_status_with_exit(handle, state, accepted_controls, 0)
    }

    fn set_status_with_exit(
        &self,
        handle: SERVICE_STATUS_HANDLE,
        state: u32,
        accepted_controls: u32,
        exit_code: u32,
    ) -> io::Result<()> {
        let status = SERVICE_STATUS {
            dwServiceType: SERVICE_WIN32_OWN_PROCESS,
            dwCurrentState: state,
            dwControlsAccepted: accepted_controls,
            dwWin32ExitCode: exit_code,
            dwServiceSpecificExitCode: 1,
            dwCheckPoint: 0,
            dwWaitHint: 0,
        };
        // SAFETY: `handle` was returned by RegisterServiceCtrlHandlerExW and
        // `status` is fully initialized for the duration of this call.
        if unsafe { SetServiceStatus(handle, &status) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn record_error(&self, error: io::Error) {
        if let Ok(mut current) = self.error.lock() {
            if current.is_none() {
                *current = Some(error);
            }
        }
    }
}

/// Observable registration state from the Windows Service Control Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    NotInstalled,
    Stopped,
    Running,
    Other,
}

/// Register and start a demand-start service. The caller must already have
/// explicit elevation; this function never attempts to elevate itself.
pub fn install(name: &str, executable: &Path, arguments: &[&str]) -> io::Result<()> {
    let service = create_service(name, executable, arguments)?;
    // SAFETY: service is a valid service handle; there are no arguments.
    if unsafe { StartServiceW(service.0, 0, std::ptr::null()) } == 0 {
        let start_error = io::Error::last_os_error();
        // SAFETY: `service` is the registration created immediately above.
        // Removing it on a failed start ensures `install` is atomic from the
        // operator's perspective: a failing helper command leaves no service
        // artefact behind for a later status probe to misinterpret.
        if unsafe { DeleteService(service.0) } == 0 {
            let rollback_error = io::Error::last_os_error();
            return Err(io::Error::new(
                start_error.kind(),
                format!(
                    "start newly registered service failed: {start_error}; rollback deletion also failed: {rollback_error}"
                ),
            ));
        }
        return Err(start_error);
    }
    Ok(())
}

/// Register a demand-start service without starting it.
///
/// This supports inspection and real SCM tests where the test command is not
/// itself a Windows service process. Production installation should use
/// [`install`], which starts the registered helper immediately.
pub fn register(name: &str, executable: &Path, arguments: &[&str]) -> io::Result<()> {
    let _service = create_service(name, executable, arguments)?;
    Ok(())
}

fn create_service(name: &str, executable: &Path, arguments: &[&str]) -> io::Result<Service> {
    let manager = Manager::open(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let name_w = wide(name)?;
    let command = service_command(executable, arguments)?;
    let command = wide(&command)?;
    // SAFETY: all UTF-16 inputs are NUL terminated and live for the call;
    // manager is a valid SCM handle owned by `Manager`.
    let handle = unsafe {
        CreateServiceW(
            manager.0,
            name_w.as_ptr(),
            name_w.as_ptr(),
            SERVICE_START | SERVICE_QUERY_STATUS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            command.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(Service(handle))
}

/// Stop, wait for, and delete an existing helper service. Service deletion is
/// explicit and does not remove any unrelated artefacts.
pub fn uninstall(name: &str) -> io::Result<()> {
    let manager = Manager::open(SC_MANAGER_CONNECT)?;
    let service = Service::open(
        &manager,
        name,
        SERVICE_QUERY_STATUS | SERVICE_STOP | DELETE_ACCESS,
    )?;
    let mut current = SERVICE_STATUS::default();
    // SAFETY: `current` is writable storage and `service` owns a valid handle.
    if unsafe { QueryServiceStatus(service.0, &mut current) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if current.dwCurrentState != SERVICE_STOPPED {
        let mut stopped = SERVICE_STATUS::default();
        // SAFETY: service is valid and stopped is writable status storage.
        if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut stopped) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_SERVICE_NOT_ACTIVE as i32) {
                return Err(error);
            }
        }
        wait_until_stopped(&service)?;
    }
    // SAFETY: service is a valid SCM service handle owned by `Service`.
    if unsafe { DeleteService(service.0) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_SERVICE_MARKED_FOR_DELETE as i32) {
            return Err(error);
        }
    }
    Ok(())
}

fn wait_until_stopped(service: &Service) -> io::Result<()> {
    const STOP_TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        let mut current = SERVICE_STATUS::default();
        // SAFETY: `current` is writable storage and `service` owns a valid handle.
        if unsafe { QueryServiceStatus(service.0, &mut current) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if current.dwCurrentState == SERVICE_STOPPED {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for helper service to stop",
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Query SCM state only. Callers must still perform an IPC round trip before
/// describing the helper as reachable.
pub fn status(name: &str) -> io::Result<ServiceStatus> {
    let manager = Manager::open(SC_MANAGER_CONNECT)?;
    let service = match Service::open(&manager, name, SERVICE_QUERY_STATUS) {
        Ok(service) => service,
        Err(error) if error.raw_os_error() == Some(1060) => return Ok(ServiceStatus::NotInstalled),
        Err(error) => return Err(error),
    };
    let mut native = SERVICE_STATUS::default();
    // SAFETY: native is valid writable storage and service is a valid SCM handle.
    if unsafe { QueryServiceStatus(service.0, &mut native) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(match native.dwCurrentState {
        SERVICE_STOPPED => ServiceStatus::Stopped,
        SERVICE_RUNNING => ServiceStatus::Running,
        _ => ServiceStatus::Other,
    })
}

struct Manager(SC_HANDLE);

impl Manager {
    fn open(access: u32) -> io::Result<Self> {
        // SAFETY: null selects the local machine and active database; access
        // contains only documented SCM access flags.
        let handle = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) };
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        // SAFETY: Manager owns this successful SCM handle exactly once.
        unsafe { CloseServiceHandle(self.0) };
    }
}

struct Service(SC_HANDLE);

impl Service {
    fn open(manager: &Manager, name: &str, access: u32) -> io::Result<Self> {
        let name_w = wide(name)?;
        // SAFETY: manager owns a valid SCM handle and name_w is NUL terminated.
        let handle = unsafe { OpenServiceW(manager.0, name_w.as_ptr(), access) };
        if handle.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: Service owns this successful SCM service handle exactly once.
        unsafe { CloseServiceHandle(self.0) };
    }
}

fn wide(value: &str) -> io::Result<Vec<u16>> {
    if value.contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
    }
    Ok(value.encode_utf16().chain(Some(0)).collect())
}

fn service_command(executable: &Path, arguments: &[&str]) -> io::Result<String> {
    let executable = executable.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "service executable path is not valid UTF-8",
        )
    })?;
    let mut command = quote_command_argument(executable)?;
    let arguments = command_arguments(arguments)?;
    if !arguments.is_empty() {
        command.push(' ');
        command.push_str(&arguments);
    }
    Ok(command)
}

fn command_arguments(arguments: &[&str]) -> io::Result<String> {
    let mut command = String::new();
    for argument in arguments {
        if !command.is_empty() {
            command.push(' ');
        }
        command.push_str(&quote_command_argument(argument)?);
    }
    Ok(command)
}

fn quote_command_argument(argument: &str) -> io::Result<String> {
    if argument.contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2).saturating_add(1)));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2)));
    quoted.push('"');
    Ok(quoted)
}

#[cfg(test)]
mod tests {
    use super::{deploy_service_executable, program_files_path, remove_service_executable};

    #[test]
    fn program_files_is_resolved_from_the_known_folder_api() {
        let path = program_files_path().expect("resolve Program Files");
        assert!(path.is_absolute());
        assert!(path.is_dir());
    }

    #[test]
    fn deployed_service_executable_is_replaced_without_touching_source() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let source = temporary.path().join("source.exe");
        let destination = temporary.path().join("installed").join("helper.exe");
        std::fs::write(&source, b"first").expect("write first source");

        deploy_service_executable(&source, &destination).expect("deploy first executable");
        assert_eq!(
            std::fs::read(&destination).expect("read first deployment"),
            b"first"
        );

        std::fs::write(&source, b"second").expect("write second source");
        deploy_service_executable(&source, &destination).expect("replace executable");
        assert_eq!(
            std::fs::read(&destination).expect("read replacement"),
            b"second"
        );
        assert_eq!(
            std::fs::read(&source).expect("read unchanged source"),
            b"second"
        );
    }

    #[test]
    fn removing_service_executable_cleans_its_empty_directory() {
        let temporary = tempfile::tempdir().expect("create temporary directory");
        let destination = temporary.path().join("installed").join("helper.exe");
        std::fs::create_dir_all(destination.parent().expect("destination parent"))
            .expect("create install directory");
        std::fs::write(&destination, b"helper").expect("write deployed executable");

        remove_service_executable(&destination).expect("remove deployed executable");

        assert!(!destination.exists());
        assert!(!destination.parent().expect("destination parent").exists());
        remove_service_executable(&destination).expect("repeat idempotent removal");
    }
}
