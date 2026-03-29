//! Windows process spawning via `std::process::Command` with `creation_flags`.
//!
//! After spawning, we duplicate the process handle, then `std::mem::forget()`
//! the std `Child` so its destructor does not close the original handle.
//! We manage the process lifecycle ourselves via `WaitForSingleObject` /
//! `GetExitCodeProcess`.

use super::{Child, ChildInner, ExitStatus, Io, ProcessGroup, SpawnConfig, SpawnError};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::process::Stdio;
use windows_sys::Win32::Foundation::{DuplicateHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, INFINITE,
};

/// `DUPLICATE_SAME_ACCESS` — documented Win32 constant (value 2).
/// Ensures the duplicated handle has the same access rights as the original.
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

/// Convert an `Io` variant to a `Stdio` for use with `std::process::Command`.
fn io_to_stdio(io: Io) -> Result<Stdio, SpawnError> {
    match io {
        Io::Inherit => Ok(Stdio::inherit()),
        Io::Null => Ok(Stdio::null()),
        Io::Pipe => Ok(Stdio::piped()),
        Io::File(f) => Ok(f.into()),
        Io::Handle(h) => {
            let new_handle = dup_handle(h)?;
            // SAFETY: new_handle is a valid handle we own from DuplicateHandle.
            Ok(unsafe { Stdio::from_raw_handle(new_handle as RawHandle) })
        }
    }
}

/// Duplicate a Windows HANDLE so the original stays owned by the caller.
fn dup_handle(h: HANDLE) -> Result<HANDLE, SpawnError> {
    let mut new_handle: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid.
    // DuplicateHandle with DUPLICATE_SAME_ACCESS creates a new handle with
    // the same access rights. bInheritHandle is FALSE (0) since we only need
    // the handle in this process.
    unsafe {
        let current = GetCurrentProcess();
        if DuplicateHandle(current, h, current, &mut new_handle, 0, 0, DUPLICATE_SAME_ACCESS) == 0 {
            return Err(SpawnError::Io(std::io::Error::last_os_error()));
        }
    }
    Ok(new_handle)
}

/// Map a spawn IO error to the appropriate `SpawnError` variant.
fn map_spawn_error(e: std::io::Error, program: &std::path::Path) -> SpawnError {
    match e.kind() {
        std::io::ErrorKind::NotFound => SpawnError::NotFound {
            path: program.to_path_buf(),
        },
        std::io::ErrorKind::PermissionDenied => SpawnError::PermissionDenied {
            path: program.to_path_buf(),
        },
        _ => SpawnError::Io(e),
    }
}

pub(super) fn spawn(config: SpawnConfig) -> Result<Child, SpawnError> {
    use std::os::windows::process::CommandExt;

    let mut cmd = std::process::Command::new(&config.program);
    cmd.args(&config.args);

    if config.env_clear {
        cmd.env_clear();
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }

    cmd.stdin(io_to_stdio(config.stdin)?);
    cmd.stdout(io_to_stdio(config.stdout)?);
    cmd.stderr(io_to_stdio(config.stderr)?);

    if matches!(config.process_group, ProcessGroup::New) {
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = cmd.spawn().map_err(|e| map_spawn_error(e, &config.program))?;

    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Duplicate the process handle before forgetting the child, since
    // std::mem::forget prevents the destructor from running (which would
    // close the original handle via Drop).
    let handle = child.as_raw_handle() as HANDLE;
    let owned_handle = dup_handle(handle)?;
    std::mem::forget(child);

    Ok(Child {
        pid,
        inner: ChildInner {
            handle: owned_handle,
        },
        stdin,
        stdout,
        stderr,
    })
}

/// Blocking wait for a child process to exit.
pub(super) fn wait_blocking(handle: HANDLE) -> Result<ExitStatus, SpawnError> {
    // SAFETY: handle is a valid process handle. WaitForSingleObject with
    // INFINITE blocks until the process exits. GetExitCodeProcess then
    // retrieves the exit code.
    unsafe {
        WaitForSingleObject(handle, INFINITE);
        let mut code: u32 = 0;
        if GetExitCodeProcess(handle, &mut code) == 0 {
            return Err(SpawnError::Io(std::io::Error::last_os_error()));
        }
        Ok(ExitStatus::from_raw(code as i32))
    }
}

/// Non-blocking wait: returns `Some` if exited, `None` if still running.
pub(super) fn try_wait(handle: HANDLE) -> Result<Option<ExitStatus>, SpawnError> {
    // SAFETY: handle is a valid process handle. WaitForSingleObject with
    // timeout 0 returns immediately.
    unsafe {
        let result = WaitForSingleObject(handle, 0);
        if result == WAIT_OBJECT_0 {
            let mut code: u32 = 0;
            if GetExitCodeProcess(handle, &mut code) == 0 {
                return Err(SpawnError::Io(std::io::Error::last_os_error()));
            }
            Ok(Some(ExitStatus::from_raw(code as i32)))
        } else {
            // WAIT_TIMEOUT or error — treat as still running.
            Ok(None)
        }
    }
}
