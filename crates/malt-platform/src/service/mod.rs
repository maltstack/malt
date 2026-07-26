//! Windows Service Control Manager operations used by the privileged helper.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{install, status, uninstall, ServiceStatus};

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    UnsupportedPlatform,
}
