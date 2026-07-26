use std::io;
use std::path::Path;

use windows_sys::Win32::System::Services::{
    CloseServiceHandle, CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW,
    QueryServiceStatus, StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE,
    SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START, SERVICE_STATUS, SERVICE_STOPPED, SERVICE_WIN32_OWN_PROCESS,
};

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
pub fn install(name: &str, executable: &Path) -> io::Result<()> {
    let manager = Manager::open(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let name_w = wide(name)?;
    let command = wide(&format!("\"{}\" --service", executable.display()))?;
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
    let service = Service(handle);
    // SAFETY: service is a valid service handle; there are no arguments.
    if unsafe { StartServiceW(service.0, 0, std::ptr::null()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Delete an existing helper service. Service deletion is explicit and does
/// not remove any unrelated artefacts.
pub fn uninstall(name: &str) -> io::Result<()> {
    let manager = Manager::open(SC_MANAGER_CONNECT)?;
    let service = Service::open(&manager, name, SERVICE_QUERY_STATUS | 0x0001_0000)?;
    // SAFETY: service is a valid SCM service handle owned by `Service`.
    if unsafe { DeleteService(service.0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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
