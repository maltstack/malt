use std::process::Command;

fn shell_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn interactive_flag_executes_script_without_repl_banner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("script.sh");
    std::fs::write(&script, "echo ok\n").expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_mash"))
        .arg("-i")
        .arg(shell_path(&script))
        .output()
        .expect("run mash");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("MASH 0.1.0"),
        "unexpected interactive banner in stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
