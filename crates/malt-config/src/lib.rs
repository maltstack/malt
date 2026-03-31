//! Vexil Store config loading for MALT.

pub mod paths;
pub mod decode;
pub use decode::VxDecoder;

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

static DAEMON_SCHEMA_SOURCE: &str =
    include_str!("../../../schemas/config/daemon.vexil");
static USER_SCHEMA_SOURCE: &str =
    include_str!("../../../schemas/config/user.vexil");

fn load_from_vx<T: VxDecoder>(
    path: &Path,
    schema_source: &str,
) -> Result<Config<T>, ConfigError> {
    let content = std::fs::read_to_string(path)?;

    let compile_result = vexil_lang::compile(schema_source);
    let schema = compile_result.compiled.ok_or_else(|| ConfigError::Invalid {
        path: path.to_path_buf(),
        reason: compile_result
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    })?;

    let values = vexil_store::parse(&content, &schema).map_err(|errors| ConfigError::Invalid {
        path: path.to_path_buf(),
        reason: errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    })?;

    let value = values.into_iter().next().ok_or_else(|| ConfigError::Invalid {
        path: path.to_path_buf(),
        reason: "config file is empty".to_string(),
    })?;

    let inner = T::from_vx_value(&value, path)?;
    Ok(Config {
        inner,
        path: Some(path.to_path_buf()),
    })
}

fn load_or_default_daemon(path: &Path) -> Result<Config<DaemonConfig>, ConfigError> {
    if !path.exists() {
        tracing::debug!(path = %path.display(), "daemon config absent, using defaults");
        return Ok(Config {
            inner: DaemonConfig::default(),
            path: None,
        });
    }
    load_from_vx(path, DAEMON_SCHEMA_SOURCE)
}

fn load_or_default_user(path: &Path) -> Result<Config<UserConfig>, ConfigError> {
    if !path.exists() {
        tracing::debug!(path = %path.display(), "user config absent, using defaults");
        return Ok(Config {
            inner: UserConfig::default(),
            path: None,
        });
    }
    load_from_vx(path, USER_SCHEMA_SOURCE)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file invalid: {}: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod load_tests {
    use super::*;

    #[test]
    fn load_or_default_daemon_absent_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.vx");
        let cfg = load_or_default_daemon(&path).unwrap();
        assert_eq!(cfg.get().max_sessions, DaemonConfig::default().max_sessions);
        assert!(cfg.path().is_none());
    }

    #[test]
    fn load_or_default_user_absent_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user.vx");
        let cfg = load_or_default_user(&path).unwrap();
        assert_eq!(cfg.get().edit_mode, UserConfig::default().edit_mode);
        assert!(cfg.path().is_none());
    }

    #[test]
    fn load_or_default_daemon_invalid_file_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.vx");
        std::fs::write(&path, "not valid vexil syntax !!!").unwrap();
        let result = load_or_default_daemon(&path);
        assert!(result.is_err());
    }

    #[test]
    fn load_or_default_daemon_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.vx");
        // .vx format: @schema directive + TypeName { field: value }
        let content = r#"@schema "malt.config.daemon"
DaemonConfig { max_sessions: 32 log_level: "debug" }
"#;
        std::fs::write(&path, content).unwrap();
        let cfg = load_or_default_daemon(&path).unwrap();
        assert_eq!(cfg.get().max_sessions, 32);
        assert_eq!(cfg.get().log_level, "debug");
        assert_eq!(cfg.path(), Some(path.as_path()));
    }

    #[test]
    fn load_or_default_user_parses_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user.vx");
        let content = r#"@schema "malt.config.user"
UserConfig { edit_mode: "vi" }
"#;
        std::fs::write(&path, content).unwrap();
        let cfg = load_or_default_user(&path).unwrap();
        assert_eq!(cfg.get().edit_mode, "vi");
        assert_eq!(cfg.path(), Some(path.as_path()));
    }
}
