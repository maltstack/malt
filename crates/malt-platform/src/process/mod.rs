//! Process spawning — SpawnConfig, Child, spawn().

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
