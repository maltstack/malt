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
        let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
        let mut mode = 0;
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }
}
