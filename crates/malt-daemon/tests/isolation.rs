//! Isolation policy failure tests that inspect observable state, rather than
//! treating an error return as proof that construction rolled back.

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_protocol::common::{IsolationPolicy, IsolationTier};

fn coordinator() -> Coordinator {
    let directory = tempfile::tempdir().expect("temporary session store");
    // Keep the directory alive for the coordinator's entire test lifetime.
    let path = directory.keep();
    let store = DebouncedStore::new(SessionStore::new(path));
    Coordinator::new(PoolConfig::default(), store)
}

#[test]
fn failed_required_containment_leaves_no_session_or_named_job_object() {
    let mut coordinator = coordinator();
    let result = coordinator.create_session_with_policy(
        Some("must-not-exist".to_string()),
        IsolationTier::Contained,
        IsolationPolicy::Required,
        None,
    );

    assert!(
        result.is_err(),
        "contained must be refused when the helper-backed HCS path cannot establish it"
    );
    assert!(
        coordinator.list_sessions().is_empty(),
        "a rejected required session must never enter coordinator state"
    );

    #[cfg(windows)]
    assert!(
        !malt_platform::isolation::job_objects::job_object_exists("malt-session-1")
            .expect("inspection of the refused session's named Job Object"),
        "Contained must fail before allocating the Job Object alias it does not provide"
    );
}

#[cfg(not(windows))]
#[test]
fn unsupported_platform_required_refuses_and_preferred_reports_bare() {
    let mut coordinator = coordinator();
    let required = coordinator.create_session_with_policy(
        Some("required".to_string()),
        IsolationTier::Restricted,
        IsolationPolicy::Required,
        None,
    );
    assert!(required.is_err());
    assert!(coordinator.list_sessions().is_empty());

    let preferred = coordinator
        .create_session_with_policy(
            Some("preferred".to_string()),
            IsolationTier::Restricted,
            IsolationPolicy::Preferred,
            None,
        )
        .expect("preferred isolation may explicitly fall back to bare");
    let session = coordinator
        .list_sessions()
        .into_iter()
        .find(|session| session.session_id == preferred)
        .expect("preferred session must be listed");
    assert_eq!(session.isolation.effective, IsolationTier::Bare);
    assert_eq!(session.isolation.requested, IsolationTier::Restricted);
    assert!(session
        .isolation
        .detail
        .unwrap_or_default()
        .contains("unavailable"));
}
