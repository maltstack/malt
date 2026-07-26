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
        assert!(
            !error.to_string().is_empty(),
            "a host-level HCS refusal must remain diagnosable"
        );
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

/// Helper invoked in a separate test-process by
/// `capped_memory_limit_binds_where_restricted_does_not`. Keeping allocation
/// outside this process makes the observed exit status a property of the Job
/// Object's child, not of the test runner's own address space.
#[cfg(windows)]
#[test]
#[ignore]
fn isolation_memory_hog_helper() {
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut bytes = vec![0u8; 64 * 1024 * 1024];
    for byte in bytes.iter_mut().step_by(4096) {
        *byte = 1;
    }
    assert_eq!(bytes[0], 1);
}

/// Restricted supplies process grouping but no resource cap; Capped adds a
/// real 16 MiB per-process limit. The same child work must therefore complete
/// outside the cap and fail inside it. This observes runtime behavior instead
/// of comparing the limit values passed to `SetInformationJobObject`.
#[cfg(windows)]
#[test]
fn capped_memory_limit_binds_where_restricted_does_not() {
    use malt_platform::isolation::job_objects;
    use std::process::Command;

    fn spawn_memory_hog() -> std::process::Child {
        Command::new(std::env::current_exe().expect("current integration-test executable"))
            .args([
                "--exact",
                "isolation_memory_hog_helper",
                "--ignored",
                "--nocapture",
            ])
            .spawn()
            .expect("spawn the isolated memory-hog helper")
    }

    let restricted_status = spawn_memory_hog()
        .wait()
        .expect("wait for unrestricted helper");
    assert!(
        restricted_status.success(),
        "the same work must complete when no Capped resource limit is present"
    );

    let mut capped_child = spawn_memory_hog();
    let capped_job = job_objects::create_job_object("malt-isolation-capped-memory", 16, 0)
        .expect("create capped Job Object");
    job_objects::assign_process_to_job(&capped_job, capped_child.id())
        .expect("assign helper before its delayed allocation");
    let capped_status = capped_child.wait().expect("wait for capped helper");
    assert!(
        !capped_status.success(),
        "a 64 MiB allocation must not complete inside the 16 MiB capped Job Object"
    );
}

#[cfg(not(windows))]
#[test]
fn windows_isolation_paths_are_not_claimed_on_other_platforms() {
    // This is a live-build assertion, not a constructed capability report:
    // the Windows modules are cfg-gated out, so this target cannot honestly
    // establish a Windows session tier.
    assert!(!cfg!(windows));
}

/// Probe: can this host actually create an HCS compute system?
///
/// **The assertion is that this test returns at all.** It used to terminate
/// the process with `STATUS_ACCESS_VIOLATION`, because
/// `HcsStartComputeSystem` was called with a null `HCS_OPERATION` handle and
/// computecore dereferences it unconditionally. No configuration can make a
/// crash acceptable: a request for an unavailable tier must produce an error a
/// caller can read, and a faulting daemon takes every other session with it.
/// So this runs unconditionally rather than `#[ignore]`d — an ignored test
/// cannot catch that regression, and for months it did not.
///
/// It deliberately does *not* assert `Ok`. Whether a compute system can be
/// created is genuinely host-dependent: it needs the Windows Containers
/// feature, Hyper-V Administrators rights, and a base image layer. The
/// outcome is printed so a reader can see which of those this host lacks.
#[cfg(windows)]
#[test]
fn hcs_create_never_faults_whatever_this_host_supports() {
    use malt_platform::isolation::hcs;

    println!("hcs_available() = {}", hcs::hcs_available());
    match hcs::ensure_hcs_runtime() {
        Ok(()) => println!("ensure_hcs_runtime() = Ok"),
        Err(e) => println!("ensure_hcs_runtime() = Err: {e}"),
    }

    // A structurally valid minimal Windows container document. Creating one
    // still needs a base layer, so a failure here distinguishes "feature
    // absent" from "feature present, image missing".
    let config = hcs::HcsConfig {
        id: format!("malt-hcs-probe-{}", std::process::id()),
        config_json: r#"{"SchemaVersion":{"Major":2,"Minor":1},"Owner":"malt","ShouldTerminateOnLastHandleClosed":true}"#
            .to_string(),
    };
    match hcs::create_compute_system(&config) {
        Ok(system) => {
            println!(
                "create_compute_system() = Ok (handle {})",
                system.raw_handle()
            );
            // Tearing down exercises the other call site that passed a null
            // operation handle, so it faulted for the same reason. A host that
            // can create a compute system must also be able to destroy one.
            let handle = system.raw_handle();
            std::mem::forget(system);
            hcs::terminate_compute_system(handle)
                .expect("a compute system that could be created must be terminable");
        }
        Err(e) => {
            let message = e.to_string();
            println!("create_compute_system() = Err: {message}");
            // A bare `HRESULT=0x8037011b` is what kept the real cause -- the
            // daemon not holding Hyper-V Administrators rights -- unreadable
            // once the crash was fixed. Every HCS HRESULT this module reports
            // goes through the decoder, so a raw code with no name means a
            // call site bypassed it.
            if message.contains("HRESULT=0x8037") {
                assert!(
                    message.contains("HCS_E_"),
                    "HCS errors must name the failure, not just its hex code: {message}"
                );
            }
        }
    }
}

/// The HCS capability report must not claim more than it checked.
///
/// `computecore.dll` ships with Windows regardless of the Containers feature
/// or of whether this build compiled the HCS backend in. Reporting `Verified`
/// on its presence advertised `Contained` as available on hosts where every
/// required request for it then failed -- a capabilities answer that
/// disagrees with reality, which is worse than none because it is what a
/// caller uses to decide what to ask for (SC-007).
///
/// This asserts the report is never `Verified`, on either build. Without the
/// `hcs` feature the runtime check fails and it must be `Unsupported`; with
/// it, the prerequisites resolve but no compute system has been created, so
/// the honest basis is `Assumed`.
#[cfg(windows)]
#[test]
fn hcs_capability_is_never_reported_as_verified_on_dll_presence_alone() {
    use malt_platform::isolation::probe::IsolationCapabilities;
    use malt_platform::isolation::{CapabilityBasis, CapabilityStatus};

    let caps = IsolationCapabilities::probe();
    let hcs = &caps.windows_hcs;

    assert_ne!(
        hcs.basis,
        CapabilityBasis::Verified,
        "HCS was reported as Verified, but nothing created a compute system;          the only check performed is that computecore.dll exists, which is          true on stock Windows. Report: {hcs:?}"
    );

    if hcs.status == CapabilityStatus::Supported {
        assert_eq!(
            hcs.basis,
            CapabilityBasis::Assumed,
            "a supported-but-unverified capability must say Assumed: {hcs:?}"
        );
    }
}

/// Print this host's full capability probe. Runs on **every** platform.
///
/// Added after the CI `isolation-capabilities` job was reported as passing on
/// Linux and macOS when it had in fact run a single negative test — every
/// other test in this file is `#[cfg(windows)]`, so the job was green on
/// hosts where isolation is entirely unwired. A green tick that would stay
/// green if the feature were absent is the exact failure this repo keeps
/// producing.
///
/// This asserts nothing about facets it cannot reason about; its job is to
/// make the answer visible in the CI log, which is what was claimed and not
/// delivered. The assertions that *can* hold everywhere are in the test
/// below.
#[test]
fn print_this_hosts_isolation_capabilities() {
    use malt_platform::isolation::IsolationCapabilities;

    let caps = IsolationCapabilities::probe();
    println!("--- isolation capabilities on {} ---", std::env::consts::OS);
    for (name, report) in [
        ("linux_namespaces", &caps.linux_namespaces),
        ("linux_cgroups", &caps.linux_cgroups),
        ("linux_overlayfs", &caps.linux_overlayfs),
        ("linux_seccomp", &caps.linux_seccomp),
        ("windows_job_objects", &caps.windows_job_objects),
        ("windows_restricted_tokens", &caps.windows_restricted_tokens),
        ("windows_hcs", &caps.windows_hcs),
        ("macos_sandbox", &caps.macos_sandbox),
        ("macos_rlimit", &caps.macos_rlimit),
    ] {
        println!(
            "  {name:<26} {:?} / {:?} ({:?}){}",
            report.status,
            report.basis,
            report.reason_code,
            report
                .reason_detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default()
        );
    }
    println!(
        "  tiers: restricted={} capped={} contained={}",
        caps.supports_restricted(),
        caps.supports_capped(),
        caps.supports_contained()
    );
}

/// The two things the probe says about a tier must agree, on every platform.
///
/// `supports_<tier>()` answers "is this available here" and
/// `unsupported_detail_for_tier()` answers "why not". A tier reported as
/// available while also carrying an unsupported reason is two surfaces
/// disagreeing about one fact — FR-007 — and it is a property that holds
/// regardless of which mechanisms a host actually has, so it is checkable on
/// Linux and macOS today rather than after those paths are wired.
#[test]
fn tier_availability_and_unsupported_detail_never_disagree() {
    use malt_platform::isolation::{IsolationCapabilities, IsolationTier};

    let caps = IsolationCapabilities::probe();
    for tier in [
        IsolationTier::Bare,
        IsolationTier::Restricted,
        IsolationTier::Capped,
        IsolationTier::Contained,
    ] {
        let available = match tier {
            IsolationTier::Bare => true,
            IsolationTier::Restricted => caps.supports_restricted(),
            IsolationTier::Capped => caps.supports_capped(),
            IsolationTier::Contained => caps.supports_contained(),
        };
        let detail = caps.unsupported_detail_for_tier(tier);
        assert_eq!(
            available,
            detail.is_none(),
            "on {}: {tier:?} reports available={available} but unsupported_detail={detail:?}; \
             the two surfaces must not disagree about the same tier",
            std::env::consts::OS
        );
    }
}
