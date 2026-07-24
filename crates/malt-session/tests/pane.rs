use malt_protocol::common::{PaneId, PaneKind};
use malt_session::pane::{CommandBlock, PaneRuntime};

fn make_block(id: u32) -> CommandBlock {
    CommandBlock {
        command_id: id,
        cmd: format!("cmd_{id}"),
        started_at: id as u64 * 100,
        finished_at: None,
        exit_code: None,
    }
}

#[test]
fn push_and_query_block() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    let block = make_block(1);
    pane.push_command_block(block.clone());

    assert_eq!(pane.block_count(), 1);
    assert_eq!(pane.command_blocks().front(), Some(&block));
}

#[test]
fn ring_buffer_eviction() {
    let mut pane = PaneRuntime::with_max_blocks(PaneId(1), PaneKind::Shell, "/home".into(), 3);

    for i in 0..5 {
        pane.push_command_block(make_block(i));
    }

    // Only the last 3 should remain
    assert_eq!(pane.block_count(), 3);
    let ids: Vec<u32> = pane.command_blocks().iter().map(|b| b.command_id).collect();
    assert_eq!(ids, vec![2, 3, 4]);
}

#[test]
fn current_block_is_most_recent() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    pane.push_command_block(make_block(10));
    pane.push_command_block(make_block(20));

    let current = pane.current_block().expect("should have a current block");
    assert_eq!(current.command_id, 20);
}

#[test]
fn empty_buffer_returns_none() {
    let pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    assert!(pane.current_block().is_none());
    assert_eq!(pane.block_count(), 0);
}

#[test]
fn finalize_sets_completion_on_the_open_block() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    pane.push_command_block(make_block(1));

    pane.finalize_current_block(500, 0);

    let block = pane.current_block().expect("should have a current block");
    assert_eq!(block.finished_at, Some(500));
    assert_eq!(block.exit_code, Some(0));
}

#[test]
fn finalize_on_an_already_completed_block_is_a_noop() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    pane.push_command_block(make_block(1));
    pane.finalize_current_block(500, 0);

    // A second finalize must not overwrite the first result -- this is what
    // keeps restored history immutable.
    pane.finalize_current_block(999, 42);

    let block = pane.current_block().expect("should have a current block");
    assert_eq!(block.finished_at, Some(500));
    assert_eq!(block.exit_code, Some(0));
}

#[test]
fn finalize_on_empty_buffer_is_a_noop() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    pane.finalize_current_block(500, 0);
    assert_eq!(pane.block_count(), 0);
}

#[test]
fn finalize_only_touches_the_newest_block() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    pane.push_command_block(make_block(1));
    pane.finalize_current_block(100, 0);
    pane.push_command_block(make_block(2));

    pane.finalize_current_block(200, 7);

    let blocks: Vec<_> = pane.command_blocks().iter().collect();
    assert_eq!(blocks[0].finished_at, Some(100));
    assert_eq!(blocks[0].exit_code, Some(0));
    assert_eq!(blocks[1].finished_at, Some(200));
    assert_eq!(blocks[1].exit_code, Some(7));
}

#[test]
fn with_blocks_seeds_history_in_order() {
    let seeded = vec![make_block(1), make_block(2), make_block(3)];
    let pane = PaneRuntime::with_blocks(PaneId(1), PaneKind::Shell, "/home".into(), 10, seeded);

    let ids: Vec<u32> = pane.command_blocks().iter().map(|b| b.command_id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!(pane.current_block().map(|b| b.command_id), Some(3));
}

#[test]
fn with_blocks_truncates_oldest_when_over_capacity() {
    let seeded = vec![make_block(1), make_block(2), make_block(3), make_block(4)];
    let pane = PaneRuntime::with_blocks(PaneId(1), PaneKind::Shell, "/home".into(), 2, seeded);

    assert_eq!(pane.block_count(), 2);
    let ids: Vec<u32> = pane.command_blocks().iter().map(|b| b.command_id).collect();
    assert_eq!(ids, vec![3, 4]);
}

#[test]
fn eviction_after_seeding_drops_the_oldest_seeded_entry_first() {
    let seeded = vec![make_block(1), make_block(2)];
    let mut pane = PaneRuntime::with_blocks(PaneId(1), PaneKind::Shell, "/home".into(), 3, seeded);

    pane.push_command_block(make_block(3));
    pane.push_command_block(make_block(4));

    assert_eq!(pane.block_count(), 3);
    let ids: Vec<u32> = pane.command_blocks().iter().map(|b| b.command_id).collect();
    assert_eq!(ids, vec![2, 3, 4]);
}

#[test]
fn with_blocks_on_empty_history_matches_a_fresh_pane() {
    let pane = PaneRuntime::with_blocks(PaneId(1), PaneKind::Shell, "/home".into(), 10, vec![]);
    assert_eq!(pane.block_count(), 0);
    assert!(pane.current_block().is_none());
}

#[test]
fn pane_state_transition() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    assert_eq!(pane.state, malt_protocol::common::PaneState::Running);

    pane.state = malt_protocol::common::PaneState::Exited;
    assert_eq!(pane.state, malt_protocol::common::PaneState::Exited);
}
