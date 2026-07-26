//! Session runtime and lifecycle state machine.

use malt_protocol::common::{
    GroupId, InputAuthority, IsolationBasis, IsolationStatus, IsolationTier, LayoutNode, PaneId,
    SessionId, SessionInfo, SessionState,
};

/// Opaque client identifier assigned by the daemon on connection.
pub type ClientId = u64;

/// Errors that can occur during session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
    #[error("session is destroyed")]
    Destroyed,
    #[error("client {0} not attached")]
    ClientNotAttached(ClientId),
    #[error("client {0} already attached")]
    ClientAlreadyAttached(ClientId),
    #[error("pane {0:?} not found")]
    PaneNotFound(PaneId),
    #[error("group policy violation: {0}")]
    PolicyViolation(String),
}

/// Runtime state for a single session.
///
/// Manages the lifecycle state machine, attached clients, input authority,
/// and pane membership. The daemon holds one `SessionRuntime` per session
/// and drives transitions through the public API.
#[derive(Debug)]
pub struct SessionRuntime {
    id: SessionId,
    name: Option<String>,
    state: SessionState,
    panes: Vec<PaneId>,
    focused_pane: PaneId,
    isolation: IsolationTier,
    group: Option<GroupId>,
    attached_clients: Vec<ClientId>,
    input_holder: Option<ClientId>,
    input_authority: InputAuthority,
    pub last_active: u64,
    layout: LayoutNode,
}

impl SessionRuntime {
    /// Create a new session in the [`SessionState::Active`] state.
    ///
    /// The session starts with a single pane which is both the focused pane
    /// and the sole leaf of the layout tree.
    pub fn new(id: SessionId, first_pane: PaneId, isolation: IsolationTier) -> Self {
        let layout = LayoutNode::Leaf {
            pane_id: first_pane.clone(),
        };
        Self {
            id,
            name: None,
            state: SessionState::Active,
            panes: vec![first_pane.clone()],
            focused_pane: first_pane,
            isolation,
            group: None,
            attached_clients: Vec::new(),
            input_holder: None,
            input_authority: InputAuthority::Exclusive,
            last_active: 0,
            layout,
        }
    }

    /// Return the current lifecycle state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// Return the session identifier.
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Return the session name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Set the session name.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// Return the session's group, if assigned.
    pub fn group(&self) -> Option<&GroupId> {
        self.group.as_ref()
    }

    /// Assign this session to a group.
    pub fn set_group(&mut self, group: Option<GroupId>) {
        self.group = group;
    }

    /// Return the session's isolation tier.
    pub fn isolation(&self) -> IsolationTier {
        self.isolation
    }

    /// Return the layout tree.
    pub fn layout(&self) -> &LayoutNode {
        &self.layout
    }

    /// Return the focused pane.
    pub fn focused_pane(&self) -> &PaneId {
        &self.focused_pane
    }

    /// Return the pane list.
    pub fn panes(&self) -> &[PaneId] {
        &self.panes
    }

    // ── Lifecycle transitions ──

    /// Attach a client to this session.
    ///
    /// If the session is [`SessionState::Dormant`], it transitions to
    /// [`SessionState::Active`]. If no input holder exists, the attaching
    /// client becomes the holder with the given authority.
    pub fn attach(
        &mut self,
        client: ClientId,
        authority: InputAuthority,
    ) -> Result<(), SessionError> {
        if self.state == SessionState::Destroyed {
            return Err(SessionError::Destroyed);
        }
        if self.attached_clients.contains(&client) {
            return Err(SessionError::ClientAlreadyAttached(client));
        }

        self.attached_clients.push(client);

        if self.state == SessionState::Dormant {
            self.state = SessionState::Active;
        }

        if self.input_holder.is_none() {
            self.input_holder = Some(client);
            self.input_authority = authority;
        }

        Ok(())
    }

    /// Detach a client from this session.
    ///
    /// If the detaching client held input, authority transfers to the next
    /// attached client (in attach order). If no clients remain, the session
    /// transitions to [`SessionState::Dormant`].
    pub fn detach(&mut self, client: ClientId) -> Result<(), SessionError> {
        if self.state == SessionState::Destroyed {
            return Err(SessionError::Destroyed);
        }

        let pos = self
            .attached_clients
            .iter()
            .position(|c| *c == client)
            .ok_or(SessionError::ClientNotAttached(client))?;

        self.attached_clients.remove(pos);

        // Transfer input if detaching client was the holder
        if self.input_holder == Some(client) {
            self.input_holder = self.attached_clients.first().copied();
        }

        if self.attached_clients.is_empty() && self.state == SessionState::Active {
            self.state = SessionState::Dormant;
        }

        Ok(())
    }

    /// Destroy this session.
    ///
    /// Valid from any non-Destroyed state.
    pub fn destroy(&mut self) -> Result<(), SessionError> {
        if self.state == SessionState::Destroyed {
            return Err(SessionError::InvalidTransition {
                from: SessionState::Destroyed,
                to: SessionState::Destroyed,
            });
        }
        self.state = SessionState::Destroyed;
        Ok(())
    }

    /// Checkpoint this session.
    ///
    /// Only valid from [`SessionState::Active`].
    pub fn checkpoint(&mut self) -> Result<(), SessionError> {
        if self.state != SessionState::Active {
            return Err(SessionError::InvalidTransition {
                from: self.state,
                to: SessionState::Checkpoint,
            });
        }
        self.state = SessionState::Checkpoint;
        Ok(())
    }

    /// Restore this session from checkpoint or dormant state.
    ///
    /// Valid from [`SessionState::Checkpoint`] or [`SessionState::Dormant`].
    pub fn restore(&mut self) -> Result<(), SessionError> {
        match self.state {
            SessionState::Checkpoint | SessionState::Dormant => {
                self.state = SessionState::Active;
                Ok(())
            }
            _ => Err(SessionError::InvalidTransition {
                from: self.state,
                to: SessionState::Active,
            }),
        }
    }

    // ── Pane management ──

    /// Add a pane to this session.
    pub fn add_pane(&mut self, pane_id: PaneId) {
        self.panes.push(pane_id);
    }

    /// Remove a pane from this session.
    pub fn remove_pane(&mut self, pane_id: PaneId) -> Result<(), SessionError> {
        let pos = self
            .panes
            .iter()
            .position(|p| *p == pane_id)
            .ok_or(SessionError::PaneNotFound(pane_id))?;
        self.panes.remove(pos);
        Ok(())
    }

    /// Set the focused pane. The pane must be a member of this session.
    pub fn set_focused(&mut self, pane_id: PaneId) -> Result<(), SessionError> {
        if !self.panes.contains(&pane_id) {
            return Err(SessionError::PaneNotFound(pane_id));
        }
        self.focused_pane = pane_id;
        Ok(())
    }

    // ── Client queries ──

    /// Return the list of attached clients.
    pub fn attached_clients(&self) -> &[ClientId] {
        &self.attached_clients
    }

    /// Return the current input holder, if any.
    pub fn input_holder(&self) -> Option<ClientId> {
        self.input_holder
    }

    /// Return the number of attached clients.
    pub fn client_count(&self) -> usize {
        self.attached_clients.len()
    }

    // ── Input authority ──

    /// Return the current input authority mode.
    pub fn input_authority(&self) -> InputAuthority {
        self.input_authority
    }

    /// Claim input authority for a client.
    ///
    /// The client must be attached. The previous holder (if any) loses input.
    pub fn claim_input(
        &mut self,
        client: ClientId,
        authority: InputAuthority,
    ) -> Result<(), SessionError> {
        if !self.attached_clients.contains(&client) {
            return Err(SessionError::ClientNotAttached(client));
        }
        self.input_holder = Some(client);
        self.input_authority = authority;
        Ok(())
    }

    // ── Persistence ──

    /// Produce a summary [`SessionInfo`] for listing.
    pub fn to_info(&self) -> SessionInfo {
        SessionInfo {
            session_id: self.id.clone(),
            name: self.name.clone(),
            pane_count: self.panes.len() as u16,
            isolation: IsolationStatus {
                effective: self.isolation,
                requested: self.isolation,
                basis: if self.isolation == IsolationTier::Bare {
                    IsolationBasis::None
                } else {
                    IsolationBasis::Assumed
                },
                mechanism: None,
                detail: None,
                _unknown: Vec::new(),
            },
            state: self.state,
            _unknown: Vec::new(),
        }
    }
}
