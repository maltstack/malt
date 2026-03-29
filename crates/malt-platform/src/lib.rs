//! OS abstractions for MALT — PTY, process spawning, signals, sockets.
//!
//! This crate is the exclusive gateway for OS interactions. No other MALT
//! crate may import `nix`, `windows-sys`, `libc`, or `std::os::unix` directly.

pub mod pty;
pub mod process;
pub mod signals;
pub mod sockets;
pub mod env;
pub mod io;
