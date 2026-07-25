//! OS abstractions for MALT — PTY, process spawning, signals, sockets.
//!
//! This crate is the exclusive gateway for OS interactions. No other MALT
//! crate may import `nix`, `windows-sys`, `libc`, or `std::os::unix` directly.

pub mod env;
pub mod fs;
pub mod io;
pub mod isolation;
pub mod process;
pub mod pty;
pub mod resource;
pub mod signals;
pub mod sockets;
pub mod vfs;

// Re-export isolation types for convenience
pub use isolation::{IsolationContext, IsolationTier};

// Re-export fs functions for convenience
pub use fs::{malt_tmp_dir, normalize_device_path, to_windows_path};

// Re-export vfs types for convenience
pub use vfs::{
    classify_virtual_dev, open_virtual_dev, try_open_virtual_dev, DevOpenMode, VirtualDevKind,
};

// Re-export resource types for convenience
pub use resource::{get_rusage, ResourceError, ResourceUsage};
