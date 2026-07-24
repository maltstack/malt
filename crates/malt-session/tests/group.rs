use malt_protocol::common::{GroupId, IsolationTier, OnEmpty, OnOom, SessionId};
use malt_protocol::persist::daemon::GroupPolicy;
use malt_session::group::GroupManager;
use malt_session::session::SessionError;

fn policy_with_max_sessions(max_sessions: u16) -> GroupPolicy {
    GroupPolicy {
        min_tier: IsolationTier::Bare,
        max_memory_mb: 1024,
        max_cpu_cores: 2,
        max_sessions,
        ttl_secs: None,
        idle_timeout_secs: None,
        on_empty: OnEmpty::Destroy,
        on_oom: OnOom::KillOffender,
        _unknown: Vec::new(),
    }
}

#[test]
fn create_group_registers_it() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(4));

    let group = mgr.get_group(GroupId(1)).expect("group should exist");
    assert_eq!(group.name, "dev");
    assert!(group.sessions.is_empty());
}

#[test]
fn get_group_missing_returns_none() {
    let mgr = GroupManager::new();
    assert!(mgr.get_group(GroupId(99)).is_none());
}

#[test]
fn remove_group_drops_it() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(4));
    mgr.remove_group(GroupId(1));
    assert!(mgr.get_group(GroupId(1)).is_none());
}

#[test]
fn add_session_within_limit_succeeds() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(2));

    mgr.add_session(GroupId(1), SessionId(10)).unwrap();
    mgr.add_session(GroupId(1), SessionId(11)).unwrap();

    let group = mgr.get_group(GroupId(1)).unwrap();
    assert_eq!(group.sessions, vec![SessionId(10), SessionId(11)]);
}

#[test]
fn add_session_to_missing_group_errors() {
    let mut mgr = GroupManager::new();
    let err = mgr.add_session(GroupId(1), SessionId(10)).unwrap_err();
    assert!(matches!(err, SessionError::PolicyViolation(_)));
}

#[test]
fn add_session_beyond_max_sessions_rejected() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(1));

    mgr.add_session(GroupId(1), SessionId(10)).unwrap();
    let err = mgr.add_session(GroupId(1), SessionId(11)).unwrap_err();
    assert!(matches!(err, SessionError::PolicyViolation(_)));

    // The rejected session must not have been added.
    let group = mgr.get_group(GroupId(1)).unwrap();
    assert_eq!(group.sessions, vec![SessionId(10)]);
}

#[test]
fn add_session_zero_max_sessions_always_rejected() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(0));
    let err = mgr.add_session(GroupId(1), SessionId(10)).unwrap_err();
    assert!(matches!(err, SessionError::PolicyViolation(_)));
}

#[test]
fn remove_session_drops_membership() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(4));
    mgr.add_session(GroupId(1), SessionId(10)).unwrap();

    mgr.remove_session(GroupId(1), SessionId(10));

    let group = mgr.get_group(GroupId(1)).unwrap();
    assert!(group.sessions.is_empty());
}

#[test]
fn remove_session_from_missing_group_is_noop() {
    let mut mgr = GroupManager::new();
    // Must not panic even though the group was never created.
    mgr.remove_session(GroupId(1), SessionId(10));
}

#[test]
fn remove_session_frees_a_slot_for_a_new_one() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(1));
    mgr.add_session(GroupId(1), SessionId(10)).unwrap();
    mgr.remove_session(GroupId(1), SessionId(10));

    // With the slot freed, a different session can now be added.
    mgr.add_session(GroupId(1), SessionId(11)).unwrap();
    let group = mgr.get_group(GroupId(1)).unwrap();
    assert_eq!(group.sessions, vec![SessionId(11)]);
}

#[test]
fn on_session_empty_returns_group_policy() {
    let mut mgr = GroupManager::new();
    let mut policy = policy_with_max_sessions(4);
    policy.on_empty = OnEmpty::Checkpoint;
    mgr.create_group(GroupId(1), "dev".to_string(), policy);

    assert_eq!(mgr.on_session_empty(GroupId(1)), Some(OnEmpty::Checkpoint));
}

#[test]
fn on_session_empty_missing_group_returns_none() {
    let mgr = GroupManager::new();
    assert_eq!(mgr.on_session_empty(GroupId(1)), None);
}

#[test]
fn on_oom_returns_group_policy() {
    let mut mgr = GroupManager::new();
    let mut policy = policy_with_max_sessions(4);
    policy.on_oom = OnOom::CheckpointThenKill;
    mgr.create_group(GroupId(1), "dev".to_string(), policy);

    assert_eq!(mgr.on_oom(GroupId(1)), Some(OnOom::CheckpointThenKill));
}

#[test]
fn on_oom_missing_group_returns_none() {
    let mgr = GroupManager::new();
    assert_eq!(mgr.on_oom(GroupId(1)), None);
}

#[test]
fn can_create_session_true_under_limit() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(2));
    mgr.add_session(GroupId(1), SessionId(10)).unwrap();

    assert!(mgr.can_create_session(GroupId(1)));
}

#[test]
fn can_create_session_false_at_limit() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(1));
    mgr.add_session(GroupId(1), SessionId(10)).unwrap();

    assert!(!mgr.can_create_session(GroupId(1)));
}

#[test]
fn can_create_session_false_for_missing_group() {
    let mgr = GroupManager::new();
    assert!(!mgr.can_create_session(GroupId(1)));
}

/// `add_session` must not silently double-count the same session id against
/// `max_sessions` — a caller retrying a message (e.g. after a timeout with no
/// visible reply) should not be able to consume two slots for one session.
#[test]
fn add_session_is_idempotent_for_the_same_session_id() {
    let mut mgr = GroupManager::new();
    mgr.create_group(GroupId(1), "dev".to_string(), policy_with_max_sessions(2));

    mgr.add_session(GroupId(1), SessionId(10)).unwrap();
    mgr.add_session(GroupId(1), SessionId(10)).unwrap();

    let group = mgr.get_group(GroupId(1)).unwrap();
    assert_eq!(
        group.sessions,
        vec![SessionId(10)],
        "adding the same session id twice must not consume two slots \
         against max_sessions"
    );
}
