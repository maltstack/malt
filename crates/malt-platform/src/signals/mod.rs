//! Signal abstraction — send, subscribe, name/number lookups.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
