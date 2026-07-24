//! Pane runtime and command block ring buffer.

use std::collections::VecDeque;

use malt_protocol::common::{PaneId, PaneKind, PaneState};

/// Default maximum number of command blocks retained per pane.
const DEFAULT_MAX_BLOCKS: usize = 1000;

/// A single command execution record within a pane.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandBlock {
    pub command_id: u32,
    pub cmd: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
}

/// Runtime state for a single content pane.
///
/// Owns a ring buffer of [`CommandBlock`]s that evicts the oldest entry when
/// capacity is reached.
#[derive(Debug)]
pub struct PaneRuntime {
    pub id: PaneId,
    pub kind: PaneKind,
    pub state: PaneState,
    pub cwd: String,
    pub title: Option<String>,
    pub pid: Option<u32>,
    command_blocks: VecDeque<CommandBlock>,
    max_blocks: usize,
}

impl PaneRuntime {
    /// Create a new pane runtime with the given identity and working directory.
    ///
    /// The pane starts in [`PaneState::Running`] with an empty command block
    /// buffer sized to the default capacity (1000).
    pub fn new(id: PaneId, kind: PaneKind, cwd: String) -> Self {
        Self {
            id,
            kind,
            state: PaneState::Running,
            cwd,
            title: None,
            pid: None,
            command_blocks: VecDeque::with_capacity(DEFAULT_MAX_BLOCKS),
            max_blocks: DEFAULT_MAX_BLOCKS,
        }
    }

    /// Create a pane runtime with a custom ring buffer capacity.
    pub fn with_max_blocks(id: PaneId, kind: PaneKind, cwd: String, max_blocks: usize) -> Self {
        Self {
            id,
            kind,
            state: PaneState::Running,
            cwd,
            title: None,
            pid: None,
            command_blocks: VecDeque::with_capacity(max_blocks),
            max_blocks,
        }
    }

    /// Create a pane runtime pre-populated with existing command blocks, for
    /// restoring persisted history.
    ///
    /// Blocks are taken in the order given (oldest first). If more than
    /// `max_blocks` are supplied, the oldest are dropped so the buffer never
    /// exceeds its capacity — the same eviction order `push_command_block`
    /// enforces at runtime.
    pub fn with_blocks(
        id: PaneId,
        kind: PaneKind,
        cwd: String,
        max_blocks: usize,
        blocks: Vec<CommandBlock>,
    ) -> Self {
        let skip = blocks.len().saturating_sub(max_blocks);
        let mut command_blocks = VecDeque::with_capacity(max_blocks);
        command_blocks.extend(blocks.into_iter().skip(skip));
        Self {
            id,
            kind,
            state: PaneState::Running,
            cwd,
            title: None,
            pid: None,
            command_blocks,
            max_blocks,
        }
    }

    /// Push a command block into the ring buffer, evicting the oldest if at capacity.
    pub fn push_command_block(&mut self, block: CommandBlock) {
        if self.command_blocks.len() >= self.max_blocks {
            self.command_blocks.pop_front();
        }
        self.command_blocks.push_back(block);
    }

    /// Finalize the newest command block, recording its completion time and
    /// exit code.
    ///
    /// Only an *open* block (one whose `finished_at` is still `None`) is
    /// finalized, and only the newest entry can be open — pushes always append
    /// and completed blocks are never reopened. Calling this when the newest
    /// block is already completed (or the buffer is empty) is a no-op, which
    /// keeps restored history immutable: blocks persisted mid-execution stay
    /// permanently un-finalized rather than being reinterpreted after restore.
    pub fn finalize_current_block(&mut self, finished_at: u64, exit_code: i32) {
        if let Some(block) = self.command_blocks.back_mut() {
            if block.finished_at.is_none() {
                block.finished_at = Some(finished_at);
                block.exit_code = Some(exit_code);
            }
        }
    }

    /// Return the most recently pushed command block, if any.
    pub fn current_block(&self) -> Option<&CommandBlock> {
        self.command_blocks.back()
    }

    /// Return the full command block ring buffer.
    pub fn command_blocks(&self) -> &VecDeque<CommandBlock> {
        &self.command_blocks
    }

    /// Return the number of command blocks currently stored.
    pub fn block_count(&self) -> usize {
        self.command_blocks.len()
    }
}
