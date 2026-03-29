//! Transport abstraction — Unix sockets, named pipes, TCP.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
