//! Windows Service Control Manager operations used by the privileged helper.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    install, is_current_process_elevated, run_elevated, run_service, status, uninstall,
    ServiceStatus, StopSignal,
};

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    UnsupportedPlatform,
}
