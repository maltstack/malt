use std::path::PathBuf;

/// Guards tests that mutate process-wide environment variables (`HOME` /
/// `USERPROFILE`). Without it, `home_dir_reflects_env_var` and
/// `home_dir_none_when_env_var_unset` can interleave under parallel test
/// execution within this binary — the same CWD_LOCK/MALT_SESSION_ID race
/// class already found and fixed elsewhere in this crate and in mash's test
/// suite (see AGENTS.md's Testing section).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
const HOME_VAR: &str = "HOME";
#[cfg(windows)]
const HOME_VAR: &str = "USERPROFILE";

#[test]
fn current_dir_matches_std_env() {
    assert_eq!(
        malt_platform::env::current_dir().unwrap(),
        std::env::current_dir().unwrap()
    );
}

#[test]
fn home_dir_reflects_env_var() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var_os(HOME_VAR);

    std::env::set_var(HOME_VAR, "C:\\fake\\home\\dir");
    assert_eq!(
        malt_platform::env::home_dir(),
        Some(PathBuf::from("C:\\fake\\home\\dir"))
    );

    match original {
        Some(v) => std::env::set_var(HOME_VAR, v),
        None => std::env::remove_var(HOME_VAR),
    }
}

#[test]
fn home_dir_none_when_env_var_unset() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var_os(HOME_VAR);

    std::env::remove_var(HOME_VAR);
    assert_eq!(malt_platform::env::home_dir(), None);

    if let Some(v) = original {
        std::env::set_var(HOME_VAR, v);
    }
}

#[test]
fn is_interactive_terminal_false_under_test_harness() {
    // cargo test never attaches stdin to a real terminal, so this should be
    // deterministically false in CI and in an interactive dev shell alike.
    assert!(!malt_platform::env::is_interactive_terminal());
}
