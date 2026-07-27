//! Windows Service Control Manager operations used by the privileged helper.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    deploy_service_executable, install, is_current_process_elevated, program_files_path, register,
    remove_service_executable, run_elevated, run_service, status, uninstall, ServiceStatus,
    StopSignal,
};

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    UnsupportedPlatform,
}
