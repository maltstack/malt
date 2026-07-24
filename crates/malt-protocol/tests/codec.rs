use malt_protocol::codec::{
    make_envelope, DOMAIN_HANDSHAKE, DOMAIN_INPUT, DOMAIN_MUX, DOMAIN_RENDER, DOMAIN_SESSION,
    DOMAIN_SHELL, DOMAIN_SYSTEM, DOMAIN_TASK, MSG_ATTACH_SESSION, MSG_CLOSE_PANE,
    MSG_COMMAND_FINISHED, MSG_COMMAND_STARTED, MSG_CREATE_SESSION, MSG_DETACH_SESSION,
    MSG_DIAGNOSTIC, MSG_ERROR, MSG_FLOAT_PANE, MSG_FOCUS_DIRECTION, MSG_FRAME_ACK, MSG_HEARTBEAT,
    MSG_HELLO, MSG_HELLO_ACK, MSG_INITIAL_STATE, MSG_INPUT_AUTHORITY_CHANGED, MSG_INPUT_CLAIM,
    MSG_KEY_EVENT, MSG_LAYOUT_CHANGED, MSG_LIST_SESSIONS, MSG_LOAD_LAYOUT, MSG_MOUSE_EVENT,
    MSG_OUTPUT_CHUNK, MSG_PANE_CREATED, MSG_PANE_DESTROYED, MSG_PLUGIN_EVENT, MSG_PROMPT_READY,
    MSG_RENDER_BATCH, MSG_RESIZE, MSG_RESIZE_SPLIT, MSG_SAVE_LAYOUT, MSG_SCROLLBACK_REQUEST,
    MSG_SCROLLBACK_RESPONSE, MSG_SESSION_LIST, MSG_SIGNAL_INPUT, MSG_SLOW_CLIENT_DISCONNECT,
    MSG_SPLIT_PANE, MSG_STRUCTURED_OUTPUT, MSG_SWAP_PANES, MSG_SYNC_REQUEST, MSG_TASK_COMPLETE,
    MSG_TASK_CREATE, MSG_TASK_STATUS, MSG_VERSION_SKEW,
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
    let env = make_envelope(DOMAIN_SYSTEM, MSG_HEARTBEAT, 0);
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

/// Every assertion here is cross-checked by hand against the `@type(N)`
/// annotation on the named message in `schemas/handshake.vexil`. If you
/// change a value in `codec.rs`, re-check it against the schema file, not
/// just this test — the two are independently maintained.
#[test]
fn handshake_message_types_match_schema() {
    assert_eq!(MSG_HELLO, 0x01); // message Hello
    assert_eq!(MSG_HELLO_ACK, 0x02); // message HelloAck
    assert_eq!(MSG_VERSION_SKEW, 0x03); // message VersionSkew
}

#[test]
fn shell_message_types_match_schema() {
    assert_eq!(MSG_COMMAND_STARTED, 0x01); // message CommandStarted
    assert_eq!(MSG_COMMAND_FINISHED, 0x02); // message CommandFinished
    assert_eq!(MSG_PROMPT_READY, 0x03); // message PromptReady
    assert_eq!(MSG_OUTPUT_CHUNK, 0x04); // message OutputChunk
}

#[test]
fn input_message_types_match_schema() {
    assert_eq!(MSG_KEY_EVENT, 0x01); // message KeyEvent
    assert_eq!(MSG_MOUSE_EVENT, 0x02); // message MouseEvent
    assert_eq!(MSG_SIGNAL_INPUT, 0x03); // message SignalInput
    assert_eq!(MSG_RESIZE, 0x04); // message Resize
}

#[test]
fn mux_message_types_match_schema() {
    assert_eq!(MSG_PANE_CREATED, 0x01); // message PaneCreated
    assert_eq!(MSG_PANE_DESTROYED, 0x02); // message PaneDestroyed
    assert_eq!(MSG_LAYOUT_CHANGED, 0x03); // message LayoutChanged
    assert_eq!(MSG_SPLIT_PANE, 0x04); // message SplitPane
    assert_eq!(MSG_CLOSE_PANE, 0x05); // message ClosePane
    assert_eq!(MSG_FLOAT_PANE, 0x06); // message FloatPane
    assert_eq!(MSG_SWAP_PANES, 0x07); // message SwapPanes
    assert_eq!(MSG_FOCUS_DIRECTION, 0x08); // message FocusDirection
    assert_eq!(MSG_RESIZE_SPLIT, 0x09); // message ResizeSplit
    assert_eq!(MSG_SAVE_LAYOUT, 0x0A); // message SaveLayout
    assert_eq!(MSG_LOAD_LAYOUT, 0x0B); // message LoadLayout
}

#[test]
fn session_message_types_match_schema() {
    assert_eq!(MSG_CREATE_SESSION, 0x01); // message CreateSession
    assert_eq!(MSG_ATTACH_SESSION, 0x02); // message AttachSession
    assert_eq!(MSG_DETACH_SESSION, 0x03); // message DetachSession
    assert_eq!(MSG_LIST_SESSIONS, 0x04); // message ListSessions
    assert_eq!(MSG_SESSION_LIST, 0x05); // message SessionList
    assert_eq!(MSG_INPUT_CLAIM, 0x06); // message InputClaim
    assert_eq!(MSG_INPUT_AUTHORITY_CHANGED, 0x07); // message InputAuthorityChanged
}

#[test]
fn task_message_types_match_schema() {
    assert_eq!(MSG_TASK_CREATE, 0x01); // message TaskCreate
    assert_eq!(MSG_TASK_STATUS, 0x02); // message TaskStatus
    assert_eq!(MSG_TASK_COMPLETE, 0x03); // message TaskComplete
}

#[test]
fn render_message_types_match_schema() {
    assert_eq!(MSG_RENDER_BATCH, 0x01); // message RenderBatch
    assert_eq!(MSG_FRAME_ACK, 0x02); // message FrameAck
    assert_eq!(MSG_INITIAL_STATE, 0x03); // message InitialState
    assert_eq!(MSG_SYNC_REQUEST, 0x04); // message SyncRequest
    assert_eq!(MSG_SLOW_CLIENT_DISCONNECT, 0x05); // message SlowClientDisconnect
    assert_eq!(MSG_SCROLLBACK_REQUEST, 0x06); // message ScrollbackRequest
    assert_eq!(MSG_SCROLLBACK_RESPONSE, 0x07); // message ScrollbackResponse
}

#[test]
fn system_message_types_match_schema() {
    assert_eq!(MSG_STRUCTURED_OUTPUT, 0x01); // message StructuredOutput
    assert_eq!(MSG_PLUGIN_EVENT, 0x02); // message PluginEvent
    assert_eq!(MSG_DIAGNOSTIC, 0x03); // message Diagnostic
    assert_eq!(MSG_HEARTBEAT, 0x04); // message Heartbeat
    assert_eq!(MSG_ERROR, 0x05); // message Error
}

/// Regression guard for the live VNP wire path: these are the exact
/// (domain, msg_type) pairs actually sent/received today by
/// `malt-tui::connection::VnpConnection` and `malt-daemon::vnp_listener`.
/// If any of these drift, the client and daemon stop being able to talk
/// to each other — this must never be a silent break.
#[test]
fn live_wire_path_constants_are_unchanged() {
    assert_eq!((DOMAIN_HANDSHAKE, MSG_HELLO), (0, 0x01));
    assert_eq!((DOMAIN_HANDSHAKE, MSG_HELLO_ACK), (0, 0x02));
    assert_eq!((DOMAIN_SESSION, MSG_ATTACH_SESSION), (4, 0x02));
    assert_eq!((DOMAIN_SESSION, MSG_DETACH_SESSION), (4, 0x03));
    assert_eq!((DOMAIN_INPUT, MSG_KEY_EVENT), (2, 0x01));
    assert_eq!((DOMAIN_INPUT, MSG_RESIZE), (2, 0x04));
    assert_eq!((DOMAIN_RENDER, MSG_RENDER_BATCH), (6, 0x01));
    assert_eq!((DOMAIN_RENDER, MSG_FRAME_ACK), (6, 0x02));
    assert_eq!((DOMAIN_RENDER, MSG_INITIAL_STATE), (6, 0x03));
}
