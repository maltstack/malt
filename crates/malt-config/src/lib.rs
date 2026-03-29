//! Vexil Store config loading for MALT.

pub mod paths;

// Generated config types from vexilc (or stubs when vexilc is unavailable).
include!(concat!(env!("OUT_DIR"), "/mod.rs"));

pub use malt::config::daemon::DaemonConfig;
pub use malt::config::user::UserConfig;

use std::path::{Path, PathBuf};

pub struct Config<T> {
    inner: T,
    path: Option<PathBuf>,
}

impl<T> Config<T> {
    pub fn get(&self) -> &T {
        &self.inner
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

pub fn load_daemon_config() -> Result<Config<DaemonConfig>, ConfigError> {
    let config_path = paths::config_dir().join("config.vx");
    load_or_default_daemon(&config_path)
}

pub fn load_user_config() -> Result<Config<UserConfig>, ConfigError> {
    let config_path = paths::config_dir().join("user.vx");
    load_or_default_user(&config_path)
}

fn load_or_default_daemon(path: &Path) -> Result<Config<DaemonConfig>, ConfigError> {
    if !path.exists() {
        return Ok(Config {
            inner: DaemonConfig::default(),
            path: None,
        });
    }
    // Full .vx loading will be integrated when vexil-store decode API
    // supports typed deserialization into generated config structs.
    // For now, return defaults even when file exists.
    Ok(Config {
        inner: DaemonConfig::default(),
        path: Some(path.to_path_buf()),
    })
}

fn load_or_default_user(path: &Path) -> Result<Config<UserConfig>, ConfigError> {
    if !path.exists() {
        return Ok(Config {
            inner: UserConfig::default(),
            path: None,
        });
    }
    // Full .vx loading will be integrated when vexil-store decode API
    // supports typed deserialization into generated config structs.
    // For now, return defaults even when file exists.
    Ok(Config {
        inner: UserConfig::default(),
        path: Some(path.to_path_buf()),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file invalid: {}: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
}
