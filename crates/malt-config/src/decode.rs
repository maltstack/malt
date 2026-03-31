use std::collections::BTreeMap;
use std::path::Path;
use vexil_store::Value;

pub trait VxDecoder: Sized {
    fn from_vx_value(value: &Value, path: &Path) -> Result<Self, crate::ConfigError>;
}

fn value_type_name(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "bool",
        Value::U8(_) => "u8",
        Value::U16(_) => "u16",
        Value::U32(_) => "u32",
        Value::U64(_) => "u64",
        Value::I8(_) => "i8",
        Value::I16(_) => "i16",
        Value::I32(_) => "i32",
        Value::I64(_) => "i64",
        Value::F32(_) => "f32",
        Value::F64(_) => "f64",
        Value::Bits { .. } => "bits",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::Rgb(_) => "rgb",
        Value::Uuid(_) => "uuid",
        Value::Timestamp(_) => "timestamp",
        Value::Hash(_) => "hash",
        Value::None => "none",
        Value::Some(_) => "some",
        Value::Array(_) => "array",
        Value::Map(_) => "map",
        Value::Ok(_) => "ok",
        Value::Err(_) => "err",
        Value::Message(_) => "message",
        Value::Enum(_) => "enum",
        Value::Flags(_) => "flags",
        Value::Union { .. } => "union",
    }
}

/// Extract an optional u32 from a message field.
/// Returns `Ok(None)` if the field is absent (caller uses its own default).
/// Returns `Err` if the field is present but the wrong type.
fn extract_u32(
    fields: &BTreeMap<String, Value>,
    name: &str,
    path: &Path,
) -> Result<Option<u32>, crate::ConfigError> {
    match fields.get(name) {
        None => Ok(None),
        Some(Value::U32(v)) => Ok(Some(*v)),
        Some(other) => Err(crate::ConfigError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "field {name}: expected u32, got {}",
                value_type_name(other)
            ),
        }),
    }
}

/// Extract an optional String from a message field.
/// Returns `Ok(None)` if the field is absent (caller uses its own default).
fn extract_string(
    fields: &BTreeMap<String, Value>,
    name: &str,
    path: &Path,
) -> Result<Option<String>, crate::ConfigError> {
    match fields.get(name) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(crate::ConfigError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "field {name}: expected string, got {}",
                value_type_name(other)
            ),
        }),
    }
}

/// Extract an `optional<string>` field (`Value::None` or `Value::Some(Value::String)`).
/// Returns `Ok(None)` if the field is absent OR is `Value::None`.
fn extract_optional_string(
    fields: &BTreeMap<String, Value>,
    name: &str,
    path: &Path,
) -> Result<Option<String>, crate::ConfigError> {
    match fields.get(name) {
        None | Some(Value::None) => Ok(None),
        Some(Value::Some(inner)) => match inner.as_ref() {
            Value::String(s) => Ok(Some(s.clone())),
            other => Err(crate::ConfigError::Invalid {
                path: path.to_path_buf(),
                reason: format!(
                    "field {name}: expected optional<string>, got some({})",
                    value_type_name(other)
                ),
            }),
        },
        Some(other) => Err(crate::ConfigError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "field {name}: expected optional<string>, got {}",
                value_type_name(other)
            ),
        }),
    }
}

/// Extract an `array<string>` field.
/// Returns `Ok(vec![])` if the field is absent.
fn extract_string_array(
    fields: &BTreeMap<String, Value>,
    name: &str,
    path: &Path,
) -> Result<Vec<String>, crate::ConfigError> {
    match fields.get(name) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut result = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item {
                    Value::String(s) => result.push(s.clone()),
                    other => {
                        return Err(crate::ConfigError::Invalid {
                            path: path.to_path_buf(),
                            reason: format!(
                                "field {name}[{i}]: expected string, got {}",
                                value_type_name(other)
                            ),
                        })
                    }
                }
            }
            Ok(result)
        }
        Some(other) => Err(crate::ConfigError::Invalid {
            path: path.to_path_buf(),
            reason: format!(
                "field {name}: expected array<string>, got {}",
                value_type_name(other)
            ),
        }),
    }
}

impl VxDecoder for crate::DaemonConfig {
    fn from_vx_value(value: &Value, path: &Path) -> Result<Self, crate::ConfigError> {
        let fields = match value {
            Value::Message(f) => f,
            other => {
                return Err(crate::ConfigError::Invalid {
                    path: path.to_path_buf(),
                    reason: format!("expected message, got {}", value_type_name(other)),
                })
            }
        };
        let defaults = crate::DaemonConfig::default();
        Ok(crate::DaemonConfig {
            socket_path: extract_optional_string(fields, "socket_path", path)?
                .or(defaults.socket_path),
            max_sessions: extract_u32(fields, "max_sessions", path)?
                .unwrap_or(defaults.max_sessions),
            default_tier: extract_string(fields, "default_tier", path)?
                .unwrap_or(defaults.default_tier),
            scrollback_lines: extract_u32(fields, "scrollback_lines", path)?
                .unwrap_or(defaults.scrollback_lines),
            log_level: extract_string(fields, "log_level", path)?
                .unwrap_or(defaults.log_level),
            persist_dir: extract_optional_string(fields, "persist_dir", path)?
                .or(defaults.persist_dir),
            persist_interval_secs: extract_u32(fields, "persist_interval_secs", path)?
                .unwrap_or(defaults.persist_interval_secs),
        })
    }
}

impl VxDecoder for crate::UserConfig {
    fn from_vx_value(value: &Value, path: &Path) -> Result<Self, crate::ConfigError> {
        let fields = match value {
            Value::Message(f) => f,
            other => {
                return Err(crate::ConfigError::Invalid {
                    path: path.to_path_buf(),
                    reason: format!("expected message, got {}", value_type_name(other)),
                })
            }
        };
        let defaults = crate::UserConfig::default();
        Ok(crate::UserConfig {
            edit_mode: extract_string(fields, "edit_mode", path)?
                .unwrap_or(defaults.edit_mode),
            theme: extract_optional_string(fields, "theme", path)?
                .or(defaults.theme),
            shell: extract_optional_string(fields, "shell", path)?
                .or(defaults.shell),
            startup_commands: {
                let v = extract_string_array(fields, "startup_commands", path)?;
                if v.is_empty() && fields.get("startup_commands").is_none() {
                    defaults.startup_commands
                } else {
                    v
                }
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DaemonConfig, UserConfig};
    use std::path::Path;

    fn dummy_path() -> &'static Path {
        Path::new("/tmp/test.vx")
    }

    fn make_daemon_msg(overrides: &[(&str, Value)]) -> Value {
        let mut fields = BTreeMap::new();
        fields.insert("socket_path".to_string(), Value::None);
        fields.insert("max_sessions".to_string(), Value::U32(64));
        fields.insert("default_tier".to_string(), Value::String("Bare".to_string()));
        fields.insert("scrollback_lines".to_string(), Value::U32(10000));
        fields.insert("log_level".to_string(), Value::String("info".to_string()));
        fields.insert("persist_dir".to_string(), Value::None);
        fields.insert("persist_interval_secs".to_string(), Value::U32(30));
        for (k, v) in overrides {
            fields.insert(k.to_string(), v.clone());
        }
        Value::Message(fields)
    }

    fn make_user_msg(overrides: &[(&str, Value)]) -> Value {
        let mut fields = BTreeMap::new();
        fields.insert("edit_mode".to_string(), Value::String("emacs".to_string()));
        fields.insert("theme".to_string(), Value::None);
        fields.insert("shell".to_string(), Value::None);
        fields.insert("startup_commands".to_string(), Value::Array(vec![]));
        for (k, v) in overrides {
            fields.insert(k.to_string(), v.clone());
        }
        Value::Message(fields)
    }

    #[test]
    fn decoder_all_daemon_fields_present() {
        let v = make_daemon_msg(&[
            ("socket_path", Value::Some(Box::new(Value::String("/tmp/malt.sock".to_string())))),
            ("max_sessions", Value::U32(32)),
            ("default_tier", Value::String("Restricted".to_string())),
            ("scrollback_lines", Value::U32(5000)),
            ("log_level", Value::String("debug".to_string())),
            ("persist_dir", Value::Some(Box::new(Value::String("/tmp/malt".to_string())))),
            ("persist_interval_secs", Value::U32(10)),
        ]);
        let cfg = DaemonConfig::from_vx_value(&v, dummy_path()).unwrap();
        assert_eq!(cfg.socket_path, Some("/tmp/malt.sock".to_string()));
        assert_eq!(cfg.max_sessions, 32);
        assert_eq!(cfg.default_tier, "Restricted");
        assert_eq!(cfg.scrollback_lines, 5000);
        assert_eq!(cfg.log_level, "debug");
        assert_eq!(cfg.persist_dir, Some("/tmp/malt".to_string()));
        assert_eq!(cfg.persist_interval_secs, 10);
    }

    #[test]
    fn decoder_missing_daemon_field_uses_default() {
        let v = Value::Message(BTreeMap::new());
        let cfg = DaemonConfig::from_vx_value(&v, dummy_path()).unwrap();
        let defaults = DaemonConfig::default();
        assert_eq!(cfg.socket_path, defaults.socket_path);
        assert_eq!(cfg.max_sessions, defaults.max_sessions);
        assert_eq!(cfg.default_tier, defaults.default_tier);
        assert_eq!(cfg.scrollback_lines, defaults.scrollback_lines);
        assert_eq!(cfg.log_level, defaults.log_level);
        assert_eq!(cfg.persist_dir, defaults.persist_dir);
        assert_eq!(cfg.persist_interval_secs, defaults.persist_interval_secs);
    }

    #[test]
    fn decoder_wrong_type_returns_error() {
        let v = make_daemon_msg(&[("max_sessions", Value::String("oops".to_string()))]);
        let err = DaemonConfig::from_vx_value(&v, dummy_path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("max_sessions"), "error should name the field: {msg}");
    }

    #[test]
    fn decoder_unknown_fields_ignored() {
        let v = make_daemon_msg(&[
            ("future_field_not_in_schema", Value::U32(999)),
            ("another_unknown", Value::String("hello".to_string())),
        ]);
        let cfg = DaemonConfig::from_vx_value(&v, dummy_path()).unwrap();
        assert_eq!(cfg.max_sessions, 64);
    }

    #[test]
    fn decoder_all_user_fields_present() {
        let v = make_user_msg(&[
            ("edit_mode", Value::String("vi".to_string())),
            ("theme", Value::Some(Box::new(Value::String("dark".to_string())))),
            ("shell", Value::Some(Box::new(Value::String("/bin/zsh".to_string())))),
            ("startup_commands", Value::Array(vec![
                Value::String("echo hello".to_string()),
                Value::String("ls -la".to_string()),
            ])),
        ]);
        let cfg = UserConfig::from_vx_value(&v, dummy_path()).unwrap();
        assert_eq!(cfg.edit_mode, "vi");
        assert_eq!(cfg.theme, Some("dark".to_string()));
        assert_eq!(cfg.shell, Some("/bin/zsh".to_string()));
        assert_eq!(cfg.startup_commands, vec!["echo hello", "ls -la"]);
    }

    #[test]
    fn decoder_missing_user_field_uses_default() {
        let v = Value::Message(BTreeMap::new());
        let cfg = UserConfig::from_vx_value(&v, dummy_path()).unwrap();
        let defaults = UserConfig::default();
        assert_eq!(cfg.edit_mode, defaults.edit_mode);
        assert_eq!(cfg.theme, defaults.theme);
        assert_eq!(cfg.shell, defaults.shell);
        assert_eq!(cfg.startup_commands, defaults.startup_commands);
    }

    #[test]
    fn decoder_startup_commands_wrong_element_type() {
        let v = make_user_msg(&[
            ("startup_commands", Value::Array(vec![Value::U32(42)])),
        ]);
        let err = UserConfig::from_vx_value(&v, dummy_path()).unwrap_err();
        assert!(err.to_string().contains("startup_commands"), "{err}");
    }
}
