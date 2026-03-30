// Dirty tracker — compares current frame commands against previous frame and emits only deltas.

use malt_protocol::render::RenderCommand;

/// Tracks previous frame state and computes minimal deltas between frames.
///
/// On each call to [`diff`](DirtyTracker::diff), the tracker compares the
/// current command list against the previous frame and returns only the
/// commands that changed, were added, or a `Clear` if elements were removed.
#[derive(Debug, Clone)]
pub struct DirtyTracker {
    previous: Vec<RenderCommand>,
}

impl DirtyTracker {
    /// Create a new tracker with no previous frame state.
    pub fn new() -> Self {
        Self {
            previous: Vec::new(),
        }
    }

    /// Compare `current` against the previous frame and return only changed commands.
    ///
    /// - First frame (no previous): emits everything.
    /// - Identical frames: returns empty vec.
    /// - Changed command at position N: emits `current[N]`.
    /// - New commands beyond previous length: emits them.
    /// - Current shorter than previous: emits `RenderCommand::Clear {}`.
    pub fn diff(&mut self, current: &[RenderCommand]) -> Vec<RenderCommand> {
        let mut delta = Vec::new();

        if self.previous.is_empty() && current.is_empty() {
            return delta;
        }

        // First frame — emit everything
        if self.previous.is_empty() {
            self.previous = current.to_vec();
            return current.to_vec();
        }

        // Compare overlapping positions
        let shared = self.previous.len().min(current.len());
        for i in 0..shared {
            if current[i] != self.previous[i] {
                delta.push(current[i].clone());
            }
        }

        // New commands beyond previous length
        for cmd in current.iter().skip(self.previous.len()) {
            delta.push(cmd.clone());
        }

        // Current shorter than previous — emit Clear to wipe removed content
        if current.len() < self.previous.len() {
            delta.push(RenderCommand::Clear {});
        }

        self.previous = current.to_vec();
        delta
    }

    /// Reset the tracker, clearing all previous frame state.
    pub fn reset(&mut self) {
        self.previous = Vec::new();
    }
}

impl Default for DirtyTracker {
    fn default() -> Self {
        Self::new()
    }
}
