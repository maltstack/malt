//! Session commands for builtins to communicate with the session executor.
//!
//! This module provides the types needed for MASH builtins to send commands
//! to the session executor for pane and layout operations.

use std::cell::RefCell;

/// Commands that can be sent from builtins to the session executor.
#[derive(Debug, Clone)]
pub enum SessionCommand {
    /// Split the current pane in the specified direction.
    PaneSplit { direction: String, ratio: f32 },
    /// Close the specified pane (or current if None).
    PaneClose { pane_id: Option<u32> },
    /// Focus the specified pane.
    PaneFocus { pane_id: u32 },
    /// Save the current layout to a named profile.
    LayoutSave { name: String },
    /// Load a named layout profile.
    LayoutLoad { name: String },
}

/// Responses from session operations.
#[derive(Debug, Clone)]
pub enum SessionResponse {
    /// Operation succeeded with optional message.
    Success { message: Option<String> },
    /// Operation failed with error message.
    Error { message: String },
    /// Response with a pane ID.
    PaneId { pane_id: u32 },
    /// Response with layout data.
    Layout { data: String },
}

// Thread-local queue for session commands from builtins.
//
// Builtins queue commands here during execution; the session executor
// drains the queue after each command and processes them.
thread_local! {
    static PENDING_COMMANDS: RefCell<Vec<SessionCommand>> = const { RefCell::new(Vec::new()) };
}

/// Queue a session command from a builtin.
///
/// # Example
/// ```
/// use mash::session::{queue_command, SessionCommand};
/// queue_command(SessionCommand::PaneSplit {
///     direction: "vertical".to_string(),
///     ratio: 0.5,
/// });
/// ```
pub fn queue_command(cmd: SessionCommand) {
    PENDING_COMMANDS.with(|queue| {
        queue.borrow_mut().push(cmd);
    });
}

/// Drain all pending session commands.
///
/// Called by the session executor after executing a command to process
/// any session operations requested by builtins.
pub fn drain_commands() -> Vec<SessionCommand> {
    PENDING_COMMANDS.with(|queue| {
        let mut cmds = queue.borrow_mut();
        let drained: Vec<SessionCommand> = cmds.drain(..).collect();
        drained
    })
}

/// Check if there are any pending session commands.
pub fn has_pending_commands() -> bool {
    PENDING_COMMANDS.with(|queue| !queue.borrow().is_empty())
}

/// Clear all pending session commands (e.g., on error recovery).
pub fn clear_commands() {
    PENDING_COMMANDS.with(|queue| {
        queue.borrow_mut().clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_and_drain() {
        // Clear any leftover commands
        clear_commands();

        // Queue some commands
        queue_command(SessionCommand::PaneSplit {
            direction: "vertical".to_string(),
            ratio: 0.5,
        });
        queue_command(SessionCommand::PaneFocus { pane_id: 42 });

        // Check pending
        assert!(has_pending_commands());

        // Drain and verify
        let cmds = drain_commands();
        assert_eq!(cmds.len(), 2);
        assert!(matches!(cmds[0], SessionCommand::PaneSplit { .. }));
        assert!(matches!(cmds[1], SessionCommand::PaneFocus { pane_id: 42 }));

        // Queue should be empty
        assert!(!has_pending_commands());
        assert!(drain_commands().is_empty());
    }

    #[test]
    fn test_clear_commands() {
        clear_commands();
        queue_command(SessionCommand::LayoutSave {
            name: "test".to_string(),
        });
        assert!(has_pending_commands());

        clear_commands();
        assert!(!has_pending_commands());
    }
}
