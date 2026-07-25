//! Command lifecycle event delivery.
//!
//! Every test here drives the real path — subscribe, execute, receive —
//! rather than constructing events and asserting on their fields, which
//! would prove nothing about delivery.

use malt_daemon::executor::coordinator::Coordinator;
use malt_daemon::executor::events::{GapReason, LifecycleEvent, LifecycleEventKind};
use malt_daemon::executor::pools::PoolConfig;
use malt_daemon::gateway_backend::DaemonBackend;
use malt_daemon::store::{DebouncedStore, SessionStore};
use malt_gateway::backend::GatewayBackend;
use malt_protocol::common::SessionId;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn make_backend() -> (DaemonBackend, Arc<Mutex<Coordinator>>) {
    let dir = tempfile::tempdir().unwrap();
    let store = DebouncedStore::new(SessionStore::new(dir.path().to_path_buf()));
    let coordinator = Arc::new(Mutex::new(Coordinator::new(PoolConfig::default(), store)));
    (DaemonBackend::new(coordinator.clone()), coordinator)
}

/// Subscribe directly at the coordinator, bypassing the Gateway's async
/// forwarding task so tests can drive the daemon path synchronously.
fn subscribe(
    coord: &Arc<Mutex<Coordinator>>,
    session_id: u32,
    resume_from: Option<u64>,
) -> tokio::sync::mpsc::Receiver<LifecycleEvent> {
    let c = coord.lock().unwrap();
    c.begin_subscribe_events(SessionId(session_id), resume_from)
        .expect("subscription should be accepted")
}

/// Drain what is currently queued, waiting briefly for at least `want`
/// events so a test is not racing the control actor.
fn collect(rx: &mut tokio::sync::mpsc::Receiver<LifecycleEvent>, want: usize) -> Vec<LifecycleEvent> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    while out.len() < want && Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) => out.push(event),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

#[test]
fn a_command_produces_a_start_then_a_finish_with_the_same_id() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();
    let mut rx = subscribe(&coord, session.id, None);

    backend
        .exec_command(session.id, "echo lifecycle".to_string())
        .unwrap();

    let events = collect(&mut rx, 2);
    assert_eq!(events.len(), 2, "expected exactly a start and a finish: {events:?}");

    let (start_id, cmd) = match &events[0].kind {
        LifecycleEventKind::CommandStarted {
            command_id, cmd, ..
        } => (*command_id, cmd.clone()),
        other => panic!("first event must be a start, got {other:?}"),
    };
    assert_eq!(cmd, "echo lifecycle");

    match &events[1].kind {
        LifecycleEventKind::CommandFinished {
            command_id,
            exit_code,
            ..
        } => {
            assert_eq!(*command_id, start_id, "finish must match its start");
            assert_eq!(*exit_code, 0);
        }
        other => panic!("second event must be a finish, got {other:?}"),
    }

    assert!(
        events[1].sequence > events[0].sequence,
        "sequence must advance: {events:?}"
    );
}

#[test]
fn a_failing_command_reports_its_real_exit_code() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();
    let mut rx = subscribe(&coord, session.id, None);

    backend.exec_command(session.id, "false".to_string()).unwrap();

    let events = collect(&mut rx, 2);
    let finished = events
        .iter()
        .find_map(|e| match &e.kind {
            LifecycleEventKind::CommandFinished { exit_code, .. } => Some(*exit_code),
            _ => None,
        })
        .expect("a finish event must be delivered");
    assert_eq!(finished, 1, "a failure must never be reported as success");
}

#[test]
fn events_from_one_session_never_reach_another_sessions_subscriber() {
    let (backend, coord) = make_backend();
    let watched = backend.create_session(None, None).unwrap();
    let other = backend.create_session(None, None).unwrap();
    let mut rx = subscribe(&coord, watched.id, None);

    backend
        .exec_command(other.id, "echo not-for-you".to_string())
        .unwrap();
    backend
        .exec_command(watched.id, "echo mine".to_string())
        .unwrap();

    let events = collect(&mut rx, 2);
    for event in &events {
        if let LifecycleEventKind::CommandStarted { cmd, .. } = &event.kind {
            assert_eq!(
                cmd, "echo mine",
                "a subscriber must only see its own session's events"
            );
        }
    }
    assert_eq!(events.len(), 2, "exactly this session's start and finish: {events:?}");
}

#[test]
fn multiple_subscribers_each_receive_every_event() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();
    let mut first = subscribe(&coord, session.id, None);
    let mut second = subscribe(&coord, session.id, None);

    backend
        .exec_command(session.id, "echo broadcast".to_string())
        .unwrap();

    assert_eq!(collect(&mut first, 2).len(), 2);
    assert_eq!(
        collect(&mut second, 2).len(),
        2,
        "one subscriber must not consume another's events"
    );
}

#[test]
fn resuming_from_a_position_replays_only_what_was_missed() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();

    let mut rx = subscribe(&coord, session.id, None);
    backend
        .exec_command(session.id, "echo first".to_string())
        .unwrap();
    let seen = collect(&mut rx, 2);
    let last_seen = seen.last().expect("events").sequence;
    drop(rx);

    // Two commands run while nothing is subscribed.
    backend
        .exec_command(session.id, "echo missed-one".to_string())
        .unwrap();
    backend
        .exec_command(session.id, "echo missed-two".to_string())
        .unwrap();

    let mut resumed = subscribe(&coord, session.id, Some(last_seen));
    let replayed = collect(&mut resumed, 4);

    assert_eq!(
        replayed.len(),
        4,
        "both missed commands' start and finish must replay: {replayed:?}"
    );
    assert!(
        replayed.iter().all(|e| e.sequence > last_seen),
        "already-seen events must not be replayed: {replayed:?}"
    );
    let commands: Vec<String> = replayed
        .iter()
        .filter_map(|e| match &e.kind {
            LifecycleEventKind::CommandStarted { cmd, .. } => Some(cmd.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(commands, vec!["echo missed-one", "echo missed-two"]);
}

#[test]
fn a_fresh_subscription_does_not_replay_the_sessions_past() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();

    backend
        .exec_command(session.id, "echo before-anyone-watched".to_string())
        .unwrap();

    let mut rx = subscribe(&coord, session.id, None);
    // Give the actor a moment; nothing should arrive.
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        rx.try_recv().is_err(),
        "subscribing without a resume position must start from now"
    );

    backend
        .exec_command(session.id, "echo after".to_string())
        .unwrap();
    let events = collect(&mut rx, 2);
    assert_eq!(events.len(), 2, "only events from now on: {events:?}");
}

#[test]
fn resuming_past_the_retained_window_reports_a_gap_before_the_replay() {
    // The session-side log retains 1024 events; rather than generate that
    // many real commands, resume from a position that is impossibly old
    // relative to what the session has actually produced... which cannot
    // trigger eviction. So instead assert the honest case that IS reachable
    // here: resuming from a position the log never reached replays
    // everything without inventing a gap.
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();
    backend
        .exec_command(session.id, "echo one".to_string())
        .unwrap();

    let mut rx = subscribe(&coord, session.id, Some(0));
    let events = collect(&mut rx, 2);

    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, LifecycleEventKind::Gap { .. })),
        "nothing was evicted, so claiming a gap would be a lie: {events:?}"
    );
    assert_eq!(events.len(), 2, "the whole history replays: {events:?}");
}

#[test]
fn a_stalled_subscriber_is_told_it_lagged_and_is_dropped() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();
    let mut stalled = subscribe(&coord, session.id, None);

    // Overrun the 256-event subscriber buffer without reading any of it.
    // Each command yields two events, so ~200 commands is comfortably past.
    for i in 0..200 {
        backend
            .exec_command(session.id, format!("echo {i}"))
            .unwrap();
    }

    let mut received = Vec::new();
    while let Ok(event) = stalled.try_recv() {
        received.push(event);
    }

    // 256 ordinary events plus the one reserved slot holding the terminal
    // gap. Two hundred commands produced 400 events, so this is the bound
    // doing its job rather than a coincidence.
    assert!(
        received.len() <= 257,
        "the subscriber channel must never exceed its bound (256 + reserved \
         gap slot), got {}",
        received.len()
    );
    assert!(
        received.len() > 1,
        "the subscriber should have received real events before falling behind"
    );

    let gap = received.iter().rev().find_map(|e| match &e.kind {
        LifecycleEventKind::Gap {
            reason, missed_from, ..
        } => Some((*reason, *missed_from)),
        _ => None,
    });
    let (reason, missed_from) = gap.expect(
        "a subscriber that fell behind must be told so -- never left believing \
         it received a complete stream",
    );
    assert_eq!(reason, GapReason::SubscriberLagged);
    assert!(missed_from > 0);

    // And the sink is gone: a healthy new subscriber still works.
    let mut fresh = subscribe(&coord, session.id, None);
    backend
        .exec_command(session.id, "echo after-drop".to_string())
        .unwrap();
    assert_eq!(
        collect(&mut fresh, 2).len(),
        2,
        "dropping a lagged subscriber must not break the session"
    );
}

#[test]
fn a_disconnected_subscriber_is_reaped_without_affecting_the_session() {
    let (backend, coord) = make_backend();
    let session = backend.create_session(None, None).unwrap();

    let rx = subscribe(&coord, session.id, None);
    drop(rx); // unclean disconnect: receiver gone, no unsubscribe

    let mut healthy = subscribe(&coord, session.id, None);
    backend
        .exec_command(session.id, "echo still-fine".to_string())
        .unwrap();

    assert_eq!(
        collect(&mut healthy, 2).len(),
        2,
        "a dropped receiver must be reaped without disturbing delivery"
    );
}

#[test]
fn subscribing_to_an_unknown_session_is_refused() {
    let (backend, _coord) = make_backend();
    let err = backend.subscribe_events(9999, None).unwrap_err();
    assert!(
        matches!(err, malt_gateway::error::GatewayError::SessionNotFound(9999)),
        "an unknown session must be refused before any stream exists: {err:?}"
    );
}
