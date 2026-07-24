use malt_protocol::codec::{
    make_envelope, DOMAIN_HANDSHAKE, DOMAIN_INPUT, DOMAIN_MUX, DOMAIN_RENDER, DOMAIN_SESSION,
    DOMAIN_SHELL, DOMAIN_SYSTEM, DOMAIN_TASK, MSG_ATTACH_SESSION, MSG_CLOSE_PANE,
    MSG_COMMAND_OUTPUT, MSG_CREATE_SESSION, MSG_DESTROY_SESSION, MSG_DETACH_SESSION, MSG_ERROR,
    MSG_FOCUS_PANE, MSG_FRAME_ACK, MSG_HELLO, MSG_HELLO_ACK, MSG_INITIAL_STATE, MSG_KEY_EVENT,
    MSG_MOUSE_EVENT, MSG_PASTE, MSG_PING, MSG_PONG, MSG_PROMPT, MSG_RENDER_BATCH, MSG_RESIZE,
    MSG_RESIZE_PANE, MSG_SHELL_READY, MSG_SHUTDOWN, MSG_SPLIT_PANE, MSG_TASK_COMPLETE,
    MSG_TASK_FAIL, MSG_TASK_PROGRESS, MSG_TASK_START,
};

#[test]
fn make_envelope_sets_fields() {
    let env = make_envelope(DOMAIN_RENDER, MSG_RENDER_BATCH, 42);
    assert_eq!(env.domain, DOMAIN_RENDER);
    assert_eq!(env.msg_type, MSG_RENDER_BATCH);
    assert_eq!(env.session_id, 42);
    assert_eq!(env.wire_version, 0);
    assert_eq!(env.msg_id, None);
}

#[test]
fn make_envelope_timestamp_is_nonzero() {
    // The envelope timestamp must reflect current wall-clock time.
    // A zero value would indicate a system clock failure on a normally running host.
    let env = make_envelope(DOMAIN_SYSTEM, MSG_PING, 0);
    assert!(
        env.timestamp > 0,
        "expected non-zero timestamp; got {}",
        env.timestamp
    );
}

#[test]
fn domain_constants_match_schema() {
    // All eight domains per architecture.md domain table
    assert_eq!(DOMAIN_HANDSHAKE, 0);
    assert_eq!(DOMAIN_SHELL, 1);
    assert_eq!(DOMAIN_INPUT, 2);
    assert_eq!(DOMAIN_MUX, 3);
    assert_eq!(DOMAIN_SESSION, 4);
    assert_eq!(DOMAIN_TASK, 5);
    assert_eq!(DOMAIN_RENDER, 6);
    assert_eq!(DOMAIN_SYSTEM, 7);
}

#[test]
fn handshake_message_types_match_schema() {
    assert_eq!(MSG_HELLO, 0x01);
    assert_eq!(MSG_HELLO_ACK, 0x02);
    assert_eq!(MSG_ERROR, 0x03);
}

#[test]
fn shell_message_types_match_schema() {
    assert_eq!(MSG_COMMAND_OUTPUT, 0x01);
    assert_eq!(MSG_SHELL_READY, 0x02);
    assert_eq!(MSG_PROMPT, 0x03);
}

#[test]
fn input_message_types_match_schema() {
    assert_eq!(MSG_KEY_EVENT, 0x01);
    assert_eq!(MSG_MOUSE_EVENT, 0x02);
    assert_eq!(MSG_PASTE, 0x03);
    assert_eq!(MSG_RESIZE, 0x04);
}

#[test]
fn mux_message_types_match_schema() {
    assert_eq!(MSG_SPLIT_PANE, 0x01);
    assert_eq!(MSG_CLOSE_PANE, 0x02);
    assert_eq!(MSG_RESIZE_PANE, 0x03);
    assert_eq!(MSG_FOCUS_PANE, 0x04);
}

#[test]
fn session_message_types_match_schema() {
    assert_eq!(MSG_CREATE_SESSION, 0x01);
    assert_eq!(MSG_ATTACH_SESSION, 0x02);
    assert_eq!(MSG_DETACH_SESSION, 0x03);
    assert_eq!(MSG_DESTROY_SESSION, 0x04);
}

#[test]
fn task_message_types_match_schema() {
    assert_eq!(MSG_TASK_START, 0x01);
    assert_eq!(MSG_TASK_PROGRESS, 0x02);
    assert_eq!(MSG_TASK_COMPLETE, 0x03);
    assert_eq!(MSG_TASK_FAIL, 0x04);
}

#[test]
fn render_message_types_match_schema() {
    assert_eq!(MSG_RENDER_BATCH, 0x01);
    assert_eq!(MSG_FRAME_ACK, 0x02);
    assert_eq!(MSG_INITIAL_STATE, 0x03);
}

#[test]
fn system_message_types_match_schema() {
    assert_eq!(MSG_PING, 0x01);
    assert_eq!(MSG_PONG, 0x02);
    assert_eq!(MSG_SHUTDOWN, 0x03);
}

#[test]
fn message_type_constants_match_schema() {
    // Spot-check a selection across all domains for the legacy test
    assert_eq!(MSG_HELLO, 0x01);
    assert_eq!(MSG_KEY_EVENT, 0x01); // input domain
    assert_eq!(MSG_RESIZE, 0x04); // input domain
    assert_eq!(MSG_ATTACH_SESSION, 0x02); // session domain
    assert_eq!(MSG_RENDER_BATCH, 0x01); // render domain
    assert_eq!(MSG_FRAME_ACK, 0x02); // render domain
    assert_eq!(MSG_INITIAL_STATE, 0x03); // render domain
}
