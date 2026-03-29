use malt_config::paths;

#[test]
fn config_dir_contains_malt() {
    let dir = paths::config_dir();
    assert!(dir.to_string_lossy().contains("malt"));
}

#[test]
fn data_dir_contains_malt() {
    let dir = paths::data_dir();
    assert!(dir.to_string_lossy().contains("malt"));
}

#[test]
fn runtime_dir_is_not_empty() {
    let dir = paths::runtime_dir();
    assert!(!dir.as_os_str().is_empty());
}
