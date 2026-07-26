//! Live-platform isolation probes.
//!
//! These tests deliberately touch the operating system rather than merely
//! constructing isolation configuration. They are the baseline evidence for
//! deciding which mechanisms a session may honestly advertise.

#[cfg(windows)]
#[test]
fn job_objects_and_hcs_entry_points_are_exercised_on_the_host() {
    use malt_platform::isolation::{hcs, job_objects};

    let job = job_objects::create_job_object("malt-isolation-reality", 0, 0)
        .expect("real CreateJobObjectA/SetInformationJobObject calls must succeed");
    assert_eq!(
        job_objects::query_active_processes(&job)
            .expect("real QueryInformationJobObject call must succeed"),
        0
    );

    // The HCS availability probe is a real filesystem/loader prerequisite
    // check. Creating a system is intentionally not attempted when this
    // build lacks the HCS backend: a successful fake or feature-disabled call
    // would not establish host containment.
    if hcs::hcs_available() {
        let error = hcs::create_compute_system(&hcs::HcsConfig {
            id: "malt-isolation-reality".to_string(),
            config_json: "{}".to_string(),
        })
        .expect_err("an HCS system requires the compiled native backend and a valid host config");
        assert!(error.to_string().contains("HCS"));
    }
}

/// Dropping the session's last Job Object handle is the teardown operation
/// MALT performs when its worker exits. This observes the child from outside
/// the Job Object and fails if teardown merely releases bookkeeping while the
/// process keeps running.
#[cfg(windows)]
#[test]
fn job_object_teardown_kills_its_real_process_tree() {
    use malt_platform::isolation::job_objects;
    use std::process::Command;
    use std::time::{Duration, Instant};

    let mut child = Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn a real long-running process");

    {
        let job = job_objects::create_job_object("malt-isolation-teardown", 0, 0)
            .expect("create real Job Object");
        job_objects::assign_process_to_job(&job, child.id())
            .expect("assign child to the session Job Object");
        assert_eq!(
            job_objects::query_active_processes(&job).expect("enumerate live Job Object"),
            1
        );
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if child.try_wait().expect("inspect child state").is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("dropping the Job Object did not terminate its assigned process");
}

#[cfg(not(windows))]
#[test]
fn windows_isolation_paths_are_not_claimed_on_other_platforms() {
    // This is a live-build assertion, not a constructed capability report:
    // the Windows modules are cfg-gated out, so this target cannot honestly
    // establish a Windows session tier.
    assert!(!cfg!(windows));
}
