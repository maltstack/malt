#![cfg(windows)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use malt_platform::service::{register, status, uninstall, ServiceStatus};

fn unique_service_name() -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    format!("MALT-Platform-Service-Test-{}-{suffix}", std::process::id())
}

#[test]
fn scm_reports_a_missing_service_as_not_installed() {
    let name = unique_service_name();
    assert_eq!(
        status(&name).expect("query SCM"),
        ServiceStatus::NotInstalled
    );
}

#[test]
#[ignore = "requires an elevated Windows session; run with --ignored to register and remove a real SCM entry"]
fn scm_registers_queries_and_removes_a_real_service() {
    let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set on Windows");
    let command = PathBuf::from(system_root).join("System32").join("cmd.exe");
    let name = unique_service_name();

    register(&name, &command, &["/c", "exit", "0"]).expect("register real SCM service");
    assert_eq!(
        status(&name).expect("query registered service"),
        ServiceStatus::Stopped
    );
    uninstall(&name).expect("remove registered service");
    assert_eq!(
        status(&name).expect("verify removal"),
        ServiceStatus::NotInstalled
    );
}
