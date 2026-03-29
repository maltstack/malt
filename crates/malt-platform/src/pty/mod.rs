//! PTY abstraction — open, read/write, resize.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
