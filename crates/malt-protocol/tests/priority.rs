use malt_protocol::priority::{priority_of, Priority};

const HANDSHAKE: u8 = 0;
const SHELL: u8 = 1;
const INPUT: u8 = 2;
const MUX: u8 = 3;
const SESSION: u8 = 4;
const TASK: u8 = 5;
const RENDER: u8 = 6;
const SYSTEM: u8 = 7;

#[test]
fn input_messages_are_critical() {
    assert_eq!(priority_of(INPUT, 0x01), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x02), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x03), Some(Priority::Critical));
    assert_eq!(priority_of(INPUT, 0x04), Some(Priority::Critical));
}

#[test]
fn handshake_messages_are_reliable() {
    assert_eq!(priority_of(HANDSHAKE, 0x01), Some(Priority::Reliable));
    assert_eq!(priority_of(HANDSHAKE, 0x02), Some(Priority::Reliable));
    assert_eq!(priority_of(HANDSHAKE, 0x03), Some(Priority::Reliable));
}

#[test]
fn shell_output_chunk_is_normal() {
    assert_eq!(priority_of(SHELL, 0x04), Some(Priority::Normal));
}

#[test]
fn shell_command_messages_are_reliable() {
    assert_eq!(priority_of(SHELL, 0x01), Some(Priority::Reliable));
    assert_eq!(priority_of(SHELL, 0x02), Some(Priority::Reliable));
    assert_eq!(priority_of(SHELL, 0x03), Some(Priority::Reliable));
}

#[test]
fn render_batch_is_high() {
    assert_eq!(priority_of(RENDER, 0x01), Some(Priority::High));
}

#[test]
fn frame_ack_is_normal() {
    assert_eq!(priority_of(RENDER, 0x02), Some(Priority::Normal));
}

#[test]
fn system_heartbeat_is_low() {
    assert_eq!(priority_of(SYSTEM, 0x04), Some(Priority::Low));
}

#[test]
fn system_error_is_reliable() {
    assert_eq!(priority_of(SYSTEM, 0x05), Some(Priority::Reliable));
}

#[test]
fn mux_all_reliable() {
    for t in 0x01..=0x0Bu8 {
        assert_eq!(priority_of(MUX, t), Some(Priority::Reliable));
    }
}

#[test]
fn session_all_reliable() {
    for t in 0x01..=0x07u8 {
        assert_eq!(priority_of(SESSION, t), Some(Priority::Reliable));
    }
}

#[test]
fn task_all_reliable() {
    for t in 0x01..=0x03u8 {
        assert_eq!(priority_of(TASK, t), Some(Priority::Reliable));
    }
}

#[test]
fn unknown_domain_returns_none() {
    assert_eq!(priority_of(15, 0x01), None);
}

#[test]
fn unknown_type_returns_none() {
    assert_eq!(priority_of(SHELL, 0x7F), None);
}
