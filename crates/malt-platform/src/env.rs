//! Environment and terminal helpers.

use std::path::PathBuf;

/// Current working directory.
pub fn current_dir() -> Result<PathBuf, std::io::Error> {
    std::env::current_dir()
}

/// User's home directory.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    { std::env::var_os("HOME").map(PathBuf::from) }
    #[cfg(windows)]
    { std::env::var_os("USERPROFILE").map(PathBuf::from) }
}

/// Whether stdin is connected to an interactive terminal.
pub fn is_interactive_terminal() -> bool {
    #[cfg(unix)]
    { nix::unistd::isatty(0).unwrap_or(false) }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, STD_INPUT_HANDLE};
        // SAFETY: STD_INPUT_HANDLE is a well-known pseudo-handle constant.
        // GetStdHandle does not require any preconditions and is always safe to
        // call; it returns INVALID_HANDLE_VALUE on failure, which GetConsoleMode
        // will then reject.
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        // SAFETY: `handle` was just obtained from GetStdHandle. `mode` is a
        // stack-allocated u32; the pointer is valid for the duration of the call.
        // GetConsoleMode returns 0 on failure (e.g. non-console handle), which
        // we map to `false` via the `!= 0` check.
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }
}
