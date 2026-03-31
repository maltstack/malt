use malt_protocol::codec::{
    make_envelope, DOMAIN_HANDSHAKE, DOMAIN_INPUT, DOMAIN_RENDER, DOMAIN_SESSION,
    MSG_HELLO, MSG_KEY_EVENT, MSG_RENDER_BATCH, MSG_FRAME_ACK, MSG_INITIAL_STATE,
    MSG_ATTACH_SESSION, MSG_RESIZE,
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
fn domain_constants_match_schema() {
    // Handshake=0, Input=2, Session=4, Render=6 per architecture.md domain table
    assert_eq!(DOMAIN_HANDSHAKE, 0);
    assert_eq!(DOMAIN_INPUT, 2);
    assert_eq!(DOMAIN_SESSION, 4);
    assert_eq!(DOMAIN_RENDER, 6);
}

#[test]
fn message_type_constants_match_schema() {
    // Per .vexil schema @type annotations
    assert_eq!(MSG_HELLO, 0x01);
    assert_eq!(MSG_KEY_EVENT, 0x01);      // input domain
    assert_eq!(MSG_RESIZE, 0x04);         // input domain
    assert_eq!(MSG_ATTACH_SESSION, 0x02); // session domain
    assert_eq!(MSG_RENDER_BATCH, 0x01);   // render domain
    assert_eq!(MSG_FRAME_ACK, 0x02);      // render domain
    assert_eq!(MSG_INITIAL_STATE, 0x03);  // render domain
}
