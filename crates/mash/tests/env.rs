use mash::env::*;

#[test]
fn empty_env_has_defaults() {
    let env = Env::empty();
    assert_eq!(env.exit_code(), 0);
    assert!(!env.is_interactive());
    assert_eq!(env.options().errexit, false);
}

#[test]
fn empty_env_has_pid() {
    let env = Env::empty();
    let pid = env.get_str("$");
    assert!(!pid.is_empty());
    assert!(pid.parse::<u32>().is_ok());
}

#[test]
fn from_os_has_path() {
    let env = Env::from_os();
    // PATH should exist on all platforms
    assert!(env.get("PATH").is_some() || env.get("Path").is_some());
}

#[test]
fn from_os_vars_are_exported() {
    let env = Env::from_os();
    if let Some(var) = env.get("PATH").or(env.get("Path")) {
        assert!(var.exported);
    }
}
