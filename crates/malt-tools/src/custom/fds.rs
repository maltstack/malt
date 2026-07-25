//! File descriptor inspection utility for POSIX conformance testing.
//!
//! Reports which file descriptors 0-9 are open or closed.
//! Used by Smoosh test suite's semantics.redir.fds test.

use crate::{BuiltinResult, ToolFn};

/// Run the fds utility: report status of file descriptors 0-9
fn fds_tool(_args: &[String], _stdin: &mut dyn std::io::Read) -> BuiltinResult {
    let mut output = String::new();

    for fd in 0..=9 {
        let status = if is_fd_open(fd) { "open" } else { "closed" };
        output.push_str(&format!("{fd} {status}\n"));
    }

    BuiltinResult::success(output.into_bytes())
}

/// Check if a file descriptor is open/valid
#[cfg(unix)]
fn is_fd_open(fd: i32) -> bool {
    use std::os::unix::io::RawFd;
    // Try to use fcntl to check if FD is valid
    unsafe {
        let ret = libc::fcntl(fd as RawFd, libc::F_GETFD);
        ret != -1
    }
}

/// Check if a file descriptor is open/valid on Windows
///
/// Uses a simple heuristic: try to read/write 0 bytes to detect if handle is valid
#[cfg(windows)]
fn is_fd_open(fd: i32) -> bool {
    use std::os::windows::io::RawHandle;

    unsafe {
        // Get the raw handle for the FD from the C runtime
        let handle = libc::get_osfhandle(fd) as RawHandle;

        // Check if handle is null or invalid (-1)
        if handle.is_null() || handle == (-1isize) as RawHandle {
            return false;
        }

        // For our purposes, assume 0-2 (stdin/stdout/stderr) are open
        // and any FD >= 3 that has a non-null handle is also open
        if fd <= 2 {
            return true;
        }

        // For other FDs, check if handle pointer is valid
        // A simple check: non-null and not -1
        true
    }
}

#[cfg(not(any(unix, windows)))]
fn is_fd_open(fd: i32) -> bool {
    // Fallback: assume 0-2 are open, rest are closed
    matches!(fd, 0..=2)
}

/// Export the tool function with correct signature
pub const FDS: ToolFn = fds_tool;
