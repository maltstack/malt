//! File descriptor inspection utility for POSIX conformance testing.
//!
//! Reports which file descriptors 0-9 are open or closed.
//! Used by Smoosh test suite's semantics.redir.fds test.

use crate::{emit, BuiltinResult, ToolFn};

/// Run the fds utility: report status of file descriptors 0-9
fn fds_tool(
    _args: &[String],
    _stdin: &mut dyn std::io::Read,
    stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut output = String::new();

    for fd in 0..=9 {
        let status = if is_fd_open(fd) { "open" } else { "closed" };
        output.push_str(&format!("{fd} {status}\n"));
    }

    emit(stdout, BuiltinResult::success(output.into_bytes()))
}

/// Check if a file descriptor is open/valid
#[cfg(unix)]
fn is_fd_open(fd: i32) -> bool {
    malt_platform::io::is_fd_open(fd)
}

/// Check if a file descriptor is open/valid on Windows
///
/// Uses a simple heuristic: try to read/write 0 bytes to detect if handle is valid
#[cfg(windows)]
fn is_fd_open(fd: i32) -> bool {
    malt_platform::io::is_fd_open(fd)
}

#[cfg(not(any(unix, windows)))]
fn is_fd_open(fd: i32) -> bool {
    // Fallback: assume 0-2 are open, rest are closed
    matches!(fd, 0..=2)
}

/// Export the tool function with correct signature
pub const FDS: ToolFn = fds_tool;
