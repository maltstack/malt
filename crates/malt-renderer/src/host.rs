// Renderer Host — orchestrates frame rendering and client dispatch.

use std::collections::HashMap;

use malt_protocol::common::{ClientCapabilities, PaneId, ResolvedPane};
use malt_protocol::frame_element::FrameElement;
use malt_protocol::render::{InitialState, RenderBatch, RenderCommand};

use crate::client_state::{AckStatus, ClientState};
use crate::dirty::DirtyTracker;
use crate::theme::ThemeResolver;
use crate::walker::{walk_frame, WalkConfig};

/// A pane's FrameElement tree.
pub struct PaneFrame {
    pub pane_id: PaneId,
    pub element: FrameElement,
}

/// A render batch targeted to a specific client.
pub struct ClientRenderBatch {
    pub client_id: u64,
    pub batch: RenderBatch,
}

/// Per-client tracking: state + dirty tracker.
struct ClientEntry {
    state: ClientState,
    dirty: DirtyTracker,
}

/// Renderer Host: orchestrates the FrameElement to RenderCommand pipeline.
///
/// For each registered client, the host walks visible pane frame trees,
/// applies dirty-diffing to emit only changed commands, and produces
/// per-client `RenderBatch` messages with sequenced frame numbers.
pub struct RendererHost {
    clients: HashMap<u64, ClientEntry>,
    #[allow(dead_code)]
    theme: ThemeResolver,
    walk_config: WalkConfig,
}

impl RendererHost {
    /// Create a new renderer host with default configuration.
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            theme: ThemeResolver::new(),
            walk_config: WalkConfig::default(),
        }
    }

    /// Register a new client with the given capabilities.
    pub fn register_client(&mut self, client_id: u64, capabilities: ClientCapabilities) {
        let entry = ClientEntry {
            state: ClientState::new(client_id, capabilities),
            dirty: DirtyTracker::new(),
        };
        self.clients.insert(client_id, entry);
    }

    /// Remove a client, dropping all associated state.
    pub fn remove_client(&mut self, client_id: u64) {
        self.clients.remove(&client_id);
    }

    /// Record a frame acknowledgement from a client.
    pub fn ack_frame(&mut self, client_id: u64, frame_seq: u64) {
        if let Some(entry) = self.clients.get_mut(&client_id) {
            entry.state.ack(frame_seq);
        }
    }

    /// Remove and return the IDs of clients that have unacked frames and
    /// haven't acked in over `SHED_TIMEOUT_MS` (10s) — see
    /// `ClientState::should_shed`. Callers must also drop any associated
    /// transport resources (e.g. the daemon's per-client render channel)
    /// for the returned IDs; `RendererHost` only owns render-side state.
    pub fn shed_stale_clients(&mut self, now_ms: u64) -> Vec<u64> {
        let stale: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, entry)| entry.state.should_shed_at(now_ms))
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            self.clients.remove(id);
        }
        stale
    }

    /// Process a frame for all registered clients.
    ///
    /// For each non-lagging client: walks all visible panes, applies dirty-diffing,
    /// and emits a `RenderBatch` with an advanced frame sequence number.
    /// Skips clients that are lagging. Skips empty deltas.
    pub fn process_frame(
        &mut self,
        panes: &[PaneFrame],
        layout: &[ResolvedPane],
    ) -> Vec<ClientRenderBatch> {
        let mut batches = Vec::new();

        // Collect client IDs to avoid borrowing issues
        let client_ids: Vec<u64> = self.clients.keys().copied().collect();

        for client_id in client_ids {
            let entry = match self.clients.get_mut(&client_id) {
                Some(e) => e,
                None => continue,
            };

            // Skip lagging clients
            if entry.state.check_status() == AckStatus::Lagging {
                continue;
            }

            // Walk all visible panes
            let commands =
                walk_visible_panes(panes, layout, entry.state.capabilities(), &self.walk_config);

            // Dirty-diff against previous frame
            let delta = entry.dirty.diff(&commands);

            // Skip empty deltas
            if delta.is_empty() {
                continue;
            }

            let seq = entry.state.advance_seq();

            batches.push(ClientRenderBatch {
                client_id,
                batch: RenderBatch {
                    frame_seq: seq,
                    commands: delta,
                    _unknown: Vec::new(),
                },
            });
        }

        batches
    }

    /// Produce a full `InitialState` snapshot for a newly attached client.
    ///
    /// This renders all visible panes without dirty-diffing, suitable for
    /// sending to a client that has just connected.
    pub fn snapshot_initial_state(
        &self,
        panes: &[PaneFrame],
        layout: &[ResolvedPane],
        client_id: u64,
    ) -> InitialState {
        let caps = self
            .clients
            .get(&client_id)
            .map(|e| e.state.capabilities().clone())
            .unwrap_or_else(default_capabilities);

        let commands = walk_visible_panes(panes, layout, &caps, &self.walk_config);

        // Build layout tree from first pane (single leaf for simplicity)
        let layout_node = if let Some(first) = layout.first() {
            malt_protocol::common::LayoutNode::Leaf {
                pane_id: first.pane_id.clone(),
            }
        } else {
            malt_protocol::common::LayoutNode::Leaf { pane_id: PaneId(0) }
        };

        InitialState {
            frame_seq: self
                .clients
                .get(&client_id)
                .map(|e| e.state.frame_seq())
                .unwrap_or(0),
            layout: layout_node,
            panes: layout.to_vec(),
            commands,
            _unknown: Vec::new(),
        }
    }

    /// Returns the number of registered clients.
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

impl Default for RendererHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Walk all visible panes and collect render commands.
fn walk_visible_panes(
    panes: &[PaneFrame],
    layout: &[ResolvedPane],
    caps: &ClientCapabilities,
    config: &WalkConfig,
) -> Vec<RenderCommand> {
    let mut commands = Vec::new();

    for pane_frame in panes {
        // Find the resolved pane in layout to get position/size
        let resolved = layout
            .iter()
            .find(|rp| rp.pane_id.0 == pane_frame.pane_id.0);
        let resolved = match resolved {
            Some(rp) if rp.visible => rp,
            _ => continue,
        };

        let result = walk_frame(
            &pane_frame.element,
            resolved.x,
            resolved.y,
            resolved.width,
            resolved.height,
            caps,
            config,
        );

        commands.extend(result.commands);
    }

    commands
}

/// Fallback capabilities when client is not registered.
fn default_capabilities() -> ClientCapabilities {
    use malt_protocol::common::{ColorDepth, ImageProtocol, UnicodeLevel};
    ClientCapabilities {
        color_depth: ColorDepth::TrueColor,
        unicode: UnicodeLevel::Full,
        image_protocol: ImageProtocol::None,
        overlay: false,
        vt_passthrough: false,
        max_fps: 60,
        _unknown: Vec::new(),
    }
}
