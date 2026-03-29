use malt_config::{load_daemon_config, load_user_config};

#[test]
fn daemon_config_defaults() {
    let config = load_daemon_config().unwrap();
    let c = config.get();
    assert_eq!(c.max_sessions, 64);
    assert_eq!(c.log_level, "info");
    assert_eq!(c.scrollback_lines, 10000);
    assert!(config.path().is_none());
}

#[test]
fn user_config_defaults() {
    let config = load_user_config().unwrap();
    let c = config.get();
    assert_eq!(c.edit_mode, "emacs");
    assert!(config.path().is_none());
}
