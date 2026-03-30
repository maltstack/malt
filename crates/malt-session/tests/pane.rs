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
fn pane_state_transition() {
    let mut pane = PaneRuntime::new(PaneId(1), PaneKind::Shell, "/home".into());
    assert_eq!(pane.state, malt_protocol::common::PaneState::Running);

    pane.state = malt_protocol::common::PaneState::Exited;
    assert_eq!(pane.state, malt_protocol::common::PaneState::Exited);
}
