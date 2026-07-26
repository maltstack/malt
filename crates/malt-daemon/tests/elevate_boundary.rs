#![cfg(windows)]

use malt_daemon::elevate_client::{
    manage_hcs_container, register_session_entitlement, run_elevated, status,
    terminate_hcs_container, HelperState,
};
use malt_platform::isolation::hcs::{create_compute_system, HcsConfig};
use malt_protocol::common::SessionId;
use malt_protocol::elevate::OutcomeKind;

/// Exercise the actual privilege boundary, not an in-process dispatch table.
///
/// This deliberately stays opt-in because it prompts through UAC and needs a
/// Windows host where an unprivileged direct HCS request is denied. A host
/// without that condition prints why it cannot establish the comparison.
#[test]
#[ignore = "requires MALT_RUN_ELEVATE_BOUNDARY=1, an installed helper, UAC consent, and Windows HCS"]
fn privilege_boundary_changes_the_outcome() {
    if std::env::var_os("MALT_RUN_ELEVATE_BOUNDARY").as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!("SKIP: set MALT_RUN_ELEVATE_BOUNDARY=1 to run the live helper comparison");
        return;
    }
    if !matches!(status(), Ok(HelperState::Reachable { .. })) {
        eprintln!("SKIP: the MALT-Elevate helper is not reachable");
        return;
    }

    let root = tempfile::tempdir().expect("create entitled storage root");
    let session_id = SessionId(80_008);
    let direct = create_compute_system(&HcsConfig {
        id: format!("malt-direct-boundary-{}", std::process::id()),
        config_json: hcs_config("direct-boundary", root.path()),
    });
    let direct_error = match direct {
        Ok(system) => {
            let _ = malt_platform::isolation::hcs::terminate_compute_system(system.raw_handle());
            panic!("direct unprivileged HCS request unexpectedly succeeded");
        }
        Err(error) => error.to_string(),
    };
    if !is_access_denied(&direct_error) {
        eprintln!(
            "SKIP: direct HCS request was not denied for privilege, so this host cannot test the boundary: {direct_error}"
        );
        return;
    }

    let malt = std::env::current_exe().ok().and_then(|path| {
        path.parent()?
            .parent()
            .map(|target| target.join("malt.exe"))
    });
    let Some(malt) = malt.filter(|path| path.is_file()) else {
        eprintln!(
            "SKIP: target/debug/malt.exe is absent; build the workspace before this live test"
        );
        return;
    };
    let pid = std::process::id().to_string();
    let exit_code = run_elevated(&malt, &["elevate", "authorize-daemon", &pid])
        .expect("request UAC approval for the live test daemon enrollment");
    assert_eq!(exit_code, 0, "elevated daemon enrollment must succeed");
    register_session_entitlement(session_id.clone(), root.path(), &[std::process::id()])
        .expect("register session entitlement with helper");

    let routed = manage_hcs_container(session_id.clone(), None, Some("malt-boundary".to_string()))
        .expect("send HCS operation through helper");
    let routed_detail = routed.detail.clone().unwrap_or_default();
    assert!(
        !is_access_denied(&routed_detail),
        "helper-routed HCS request must not fail for the daemon's privilege denial: {routed:?}"
    );
    if routed.kind == OutcomeKind::Performed {
        let id = String::from_utf8(
            routed
                .payload
                .expect("performed operation has an id payload"),
        )
        .expect("helper compute-system id is UTF-8");
        let teardown = terminate_hcs_container(session_id, id)
            .expect("send helper-owned compute-system teardown");
        assert_eq!(teardown.kind, OutcomeKind::Performed, "{teardown:?}");
    }
}

fn is_access_denied(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("0x80070005")
        || lower.contains("hcs_e_access_denied")
}

fn hcs_config(id: &str, root: &std::path::Path) -> String {
    let root = root.to_string_lossy().replace('\\', r#"\\"#);
    format!(
        r#"{{"SchemaVersion":{{"Major":2,"Minor":1}},"Owner":"malt-{id}","ShouldTerminateOnLastHandleClosed":true,"Container":{{"Storage":{{"Path":"{root}","Layers":[]}},"GuestOs":{{"HostName":"malt"}}}}}}"#
    )
}
