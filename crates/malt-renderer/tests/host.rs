use malt_protocol::common::{
    ClientCapabilities, ColorDepth, ImageProtocol, PaneId, ResolvedPane, ResolvedStyle,
    UnicodeLevel,
};
use malt_protocol::frame_element::FrameElement;
use malt_renderer::host::{PaneFrame, RendererHost};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_millis() as u64
}

fn default_style() -> ResolvedStyle {
    ResolvedStyle {
        fg: (204, 204, 204),
        bg: (0, 0, 0),
        bold: false,
        italic: false,
        underline: false,
        dim: false,
        strikethrough: false,
        reverse: false,
        blink: false,
        token_name: None,
        _unknown: Vec::new(),
    }
}

fn full_caps() -> ClientCapabilities {
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: true,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}

fn make_pane(id: u32, x: u16, y: u16, w: u16, h: u16) -> ResolvedPane {
    ResolvedPane {
        pane_id: PaneId(id),
        x,
        y,
        width: w,
        height: h,
        focused: id == 1,
        visible: true,
        z_order: 0,
        tab_context: None,
        _unknown: Vec::new(),
    }
}

#[test]
fn register_client_and_render() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "hello".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];

    let batches = host.process_frame(&panes, &layout);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].client_id, 1);
    assert_eq!(batches[0].batch.frame_seq, 1);
    assert!(!batches[0].batch.commands.is_empty());
}

#[test]
fn lagging_client_skipped() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let layout = [make_pane(1, 0, 0, 80, 24)];

    // Send 30 frames without acking — use different content each time
    // so dirty tracker produces deltas and frame_seq advances to 30
    for i in 0..30 {
        let panes = [PaneFrame {
            pane_id: PaneId(1),
            element: FrameElement::Text {
                text: format!("frame-{i}"),
                style: Box::new(default_style()),
            },
        }];
        host.process_frame(&panes, &layout);
    }

    // 31st frame — client should be lagging (30 unacked), no batches produced
    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "frame-30".into(),
            style: Box::new(default_style()),
        },
    }];
    let batches = host.process_frame(&panes, &layout);
    assert!(batches.is_empty());
}

#[test]
fn ack_resumes_production() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let layout = [make_pane(1, 0, 0, 80, 24)];

    // Send 30 frames without acking — use different content each time
    // to ensure dirty tracker produces a delta and frame_seq advances
    for i in 0..30 {
        let panes = [PaneFrame {
            pane_id: PaneId(1),
            element: FrameElement::Text {
                text: format!("frame-{i}"),
                style: Box::new(default_style()),
            },
        }];
        host.process_frame(&panes, &layout);
    }

    // Ack all frames
    host.ack_frame(1, 30);

    // Next frame should produce a batch
    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "resumed".into(),
            style: Box::new(default_style()),
        },
    }];
    let batches = host.process_frame(&panes, &layout);
    assert_eq!(batches.len(), 1);
}

#[test]
fn unregistered_client_no_batches() {
    let mut host = RendererHost::new();

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "data".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];

    let batches = host.process_frame(&panes, &layout);
    assert!(batches.is_empty());
}

#[test]
fn initial_state_snapshot() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "snapshot".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];

    let state = host.snapshot_initial_state(&panes, &layout, 1);
    assert!(!state.commands.is_empty());
    assert_eq!(state.panes.len(), 1);
    assert_eq!(state.panes[0].pane_id.0, 1);
}

#[test]
fn shed_stale_clients_removes_unacked_client_after_timeout() {
    let start = now_ms();
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "data".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];
    // Advance the client's frame_seq so it has an unacked frame pending.
    host.process_frame(&panes, &layout);
    assert_eq!(host.client_count(), 1);

    // Not enough time has passed — client should survive.
    let shed = host.shed_stale_clients(start + 1_000);
    assert!(shed.is_empty());
    assert_eq!(host.client_count(), 1);

    // 10s+ since registration with the frame still unacked — client is shed.
    let shed = host.shed_stale_clients(start + 10_001);
    assert_eq!(shed, vec![1]);
    assert_eq!(host.client_count(), 0);
}

#[test]
fn shed_stale_clients_spares_fully_acked_client() {
    let start = now_ms();
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "data".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];
    host.process_frame(&panes, &layout);
    host.ack_frame(1, 1);

    // Even long after registration, a client with zero unacked frames is
    // never shed — should_shed requires unacked_count() > 0.
    let shed = host.shed_stale_clients(start + 60_000);
    assert!(shed.is_empty());
    assert_eq!(host.client_count(), 1);
}

#[test]
fn shed_stale_clients_spares_client_with_no_frames_sent() {
    let start = now_ms();
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());

    // No process_frame call at all — unacked_count() is 0 from the start.
    let shed = host.shed_stale_clients(start + 60_000);
    assert!(shed.is_empty());
    assert_eq!(host.client_count(), 1);
}

#[test]
fn remove_client() {
    let mut host = RendererHost::new();
    host.register_client(1, full_caps());
    host.remove_client(1);

    let panes = [PaneFrame {
        pane_id: PaneId(1),
        element: FrameElement::Text {
            text: "data".into(),
            style: Box::new(default_style()),
        },
    }];
    let layout = [make_pane(1, 0, 0, 80, 24)];

    let batches = host.process_frame(&panes, &layout);
    assert!(batches.is_empty());
}
