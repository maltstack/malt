//! I/O utilities — pipe creation, tty detection, executable check, handle conversion.

use std::path::Path;

/// Returns `true` if the given file descriptor refers to a terminal.
///
/// On Windows this always returns `false` (PTY handles are not classic ttys).
pub fn is_tty(fd: i32) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::RawFd;
        // nix::unistd::isatty returns Ok(true) for a tty, Ok(false)/Err for non-tty/invalid fd.
        nix::unistd::isatty(fd as RawFd).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        false
    }
}

/// Returns `true` if `path` is a file with executable bits set.
///
/// On Unix, checks the execute bits via `PermissionsExt`.
/// On Windows, returns `true` for any regular file (PATHEXT extension matching
/// handles executability at the call site).
pub fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Convert any type that yields a raw Windows `HANDLE` into an owned `File`.
///
/// This centralises the `FromRawHandle` unsafe in malt-platform so no other
/// crate needs to import `std::os::windows::io`.
#[cfg(windows)]
pub fn into_file<T: std::os::windows::io::IntoRawHandle>(handle: T) -> std::fs::File {
    use std::os::windows::io::FromRawHandle;
    // SAFETY: `handle` must be a valid, open Windows HANDLE — this is a
    // precondition the caller asserts by passing a type that implements
    // `IntoRawHandle`. `into_raw_handle()` transfers ownership out of `T`,
    // making this the sole owner. `File` will close the handle on drop.
    unsafe { std::fs::File::from_raw_handle(handle.into_raw_handle()) }
}

/// Create an anonymous OS pipe.
/// Returns `(read_end, write_end)` as owned file handles.
pub fn create_pipe() -> Result<(std::fs::File, std::fs::File), std::io::Error> {
    #[cfg(unix)]
    {
        let (read_fd, write_fd) = nix::unistd::pipe()
            .map_err(std::io::Error::from)?;
        Ok((std::fs::File::from(read_fd), std::fs::File::from(write_fd)))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Pipes::CreatePipe;
        use std::os::windows::io::FromRawHandle;

        let mut read_h = INVALID_HANDLE_VALUE;
        let mut write_h = INVALID_HANDLE_VALUE;
        // SAFETY: CreatePipe is called with valid pointers to HANDLE locals and a
        // null SECURITY_ATTRIBUTES pointer (default security, non-inheritable). A
        // buffer size of 0 selects the system default. No preconditions beyond
        // valid writable pointers, which read_h and write_h satisfy.
        let ok = unsafe { CreatePipe(&mut read_h, &mut write_h, std::ptr::null(), 0) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: CreatePipe returns two new valid handles on success.
        // We immediately wrap them in File for RAII ownership.
        Ok(unsafe {
            (
                std::fs::File::from_raw_handle(read_h as *mut _),
                std::fs::File::from_raw_handle(write_h as *mut _),
            )
        })
    }
}
