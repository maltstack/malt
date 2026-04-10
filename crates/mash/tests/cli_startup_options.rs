use std::process::Command;

fn mash_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mash")
}

#[test]
fn startup_u_enables_nounset_for_c_command() {
    let out = Command::new(mash_bin())
        .args(["-u", "-c", ": ${UNSET_FOR_STARTUP_OPT_TEST}"])
        .output()
        .expect("run mash");

    assert!(
        !out.status.success(),
        "expected nounset failure, got status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("undefined variable: UNSET_FOR_STARTUP_OPT_TEST")
    );
}

#[test]
fn startup_plus_u_disables_nounset_for_c_command() {
    let out = Command::new(mash_bin())
        .args(["-u", "+u", "-c", ": ${UNSET_FOR_STARTUP_OPT_TEST}"])
        .output()
        .expect("run mash");

    assert!(
        out.status.success(),
        "expected nounset disabled, got status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[cfg(not(windows))]
fn startup_u_keeps_ppid_available_for_c_command() {
    let out = Command::new(mash_bin())
        .args(["-u", "-c", "printf '%s\\n' \"$PPID\""])
        .output()
        .expect("run mash");

    assert!(
        out.status.success(),
        "expected PPID to remain available under -u, got status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let ppid = String::from_utf8_lossy(&out.stdout);
    let trimmed = ppid.trim();
    assert!(
        !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit()) && trimmed != "0",
        "stdout={} stderr={}",
        ppid,
        String::from_utf8_lossy(&out.stderr)
    );
}
