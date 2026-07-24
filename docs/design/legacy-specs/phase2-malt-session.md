# Phase 2: `malt-session` — Session Lifecycle + Pane Runtime

## Goal

Build the session management crate — session lifecycle state machine, pane runtime with command block ring buffer, group policy enforcement, and input authority state machine. Testable in isolation without a running daemon.

## Architecture

L1 crate depending only on `malt-protocol` for schema types. Pure logic — no I/O, no platform. The daemon (Phase 3) uses these types and state machines to manage sessions. All state transitions are validated by the crate; invalid transitions return errors.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-session\src\lib.rs` (328 lines). MALT version is larger due to lifecycle state machine, group policies, and input authority.

---

## Crate Structure

```
orix/malt/crates/malt-session/
  Cargo.toml
  src/
    lib.rs              # Re-exports
    session.rs          # SessionRuntime, lifecycle state machine
    pane.rs             # PaneRuntime, CommandBlock, ring buffer
    group.rs            # GroupManager, policy enforcement
    authority.rs        # Input authority state machine
  tests/
    session.rs
    pane.rs
    group.rs
    authority.rs
```

---

## Dependencies

```toml
[dependencies]
malt-protocol = { path = "../malt-protocol" }
thiserror = "2"
```

---

## Types

### PaneRuntime

```rust
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

pub struct CommandBlock {
    pub command_id: u32,
    pub cmd: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
}
```

**Ring buffer:** `push_command_block()` evicts oldest when at capacity. Default 1000 blocks. `current_block()` returns the most recent.

### SessionRuntime

```rust
pub struct SessionRuntime {
    pub id: SessionId,
    pub name: Option<String>,
    state: SessionState,
    pub panes: Vec<PaneId>,
    pub focused_pane: PaneId,
    pub isolation: IsolationTier,
    pub group: Option<GroupId>,
    attached_clients: Vec<ClientId>,
    input_holder: Option<ClientId>,
    input_authority: InputAuthority,
    pub last_active: u64,
    pub layout: LayoutNode,
}

pub type ClientId = u64;
```

### SessionError

```rust
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid transition: {from:?} → {to:?}")]
    InvalidTransition { from: SessionState, to: SessionState },
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
```

---

## Session Lifecycle State Machine

Valid transitions:

```
Active → Dormant      (last client detaches)
Active → Destroyed    (explicit destroy)
Active → Checkpoint   (explicit checkpoint, Contained tier only)
Dormant → Active      (client attaches)
Dormant → Destroyed   (explicit destroy)
Checkpoint → Active   (restore)
Checkpoint → Destroyed (explicit destroy)
```

All other transitions → `SessionError::InvalidTransition`.

### Public API

```rust
impl SessionRuntime {
    pub fn new(id: SessionId, first_pane: PaneId, isolation: IsolationTier) -> Self;
    pub fn state(&self) -> SessionState;

    // Lifecycle
    pub fn attach(&mut self, client: ClientId, authority: InputAuthority) -> Result<(), SessionError>;
    pub fn detach(&mut self, client: ClientId) -> Result<(), SessionError>;
    pub fn destroy(&mut self) -> Result<(), SessionError>;
    pub fn checkpoint(&mut self) -> Result<(), SessionError>;
    pub fn restore(&mut self) -> Result<(), SessionError>;

    // Pane management
    pub fn add_pane(&mut self, pane_id: PaneId);
    pub fn remove_pane(&mut self, pane_id: PaneId) -> Result<(), SessionError>;
    pub fn set_focused(&mut self, pane_id: PaneId) -> Result<(), SessionError>;

    // Client queries
    pub fn attached_clients(&self) -> &[ClientId];
    pub fn input_holder(&self) -> Option<ClientId>;
    pub fn client_count(&self) -> usize;

    // Persistence
    pub fn to_info(&self) -> SessionInfo;
    pub fn to_persisted(&self, panes: &HashMap<PaneId, PaneRuntime>) -> PersistedSession;
    pub fn from_persisted(persisted: &PersistedSession) -> Self;
}
```

### Transition Details

**attach:** Add client. If state is Dormant → transition to Active. Set input authority. If no holder, this client becomes holder.

**detach:** Remove client. Transfer input to next attached client (if any). If no clients remain → transition to Dormant. Update `last_active`.

**destroy:** Transition to Destroyed from any non-Destroyed state.

**checkpoint:** Only from Active. Transition to Checkpoint.

**restore:** Only from Checkpoint or Dormant. Transition to Active.

---

## Input Authority

```rust
impl SessionRuntime {
    pub fn claim_input(&mut self, client: ClientId, authority: InputAuthority) -> Result<(), SessionError>;
    pub fn input_authority(&self) -> InputAuthority;
}
```

**claim_input:** Client must be attached. Sets new holder and authority mode. Previous holder loses input.

**transfer_on_detach:** When the input holder detaches, authority transfers to the next client in attach order. If no clients remain, holder becomes None.

---

## Group Policy

```rust
pub struct GroupManager {
    groups: HashMap<GroupId, GroupRuntime>,
}

pub struct GroupRuntime {
    pub id: GroupId,
    pub name: String,
    pub policy: GroupPolicy,
    pub sessions: Vec<SessionId>,
}

impl GroupManager {
    pub fn new() -> Self;
    pub fn create_group(&mut self, id: GroupId, name: String, policy: GroupPolicy);
    pub fn remove_group(&mut self, id: GroupId);
    pub fn add_session(&mut self, group_id: GroupId, session_id: SessionId) -> Result<(), SessionError>;
    pub fn remove_session(&mut self, group_id: GroupId, session_id: SessionId);
    pub fn on_session_empty(&self, group_id: GroupId) -> Option<OnEmpty>;
    pub fn on_oom(&self, group_id: GroupId) -> Option<OnOom>;
    pub fn can_create_session(&self, group_id: GroupId) -> bool;
    pub fn get_group(&self, id: GroupId) -> Option<&GroupRuntime>;
}
```

`can_create_session` checks `policy.max_sessions` against current session count. `on_session_empty` and `on_oom` return the policy action for the daemon to execute.

---

## Testing Strategy

### Session lifecycle (8 tests)
- Create → state is Active
- Attach client → client listed
- Detach last client → state becomes Dormant
- Attach to Dormant → state becomes Active
- Destroy from Active → Destroyed
- Destroy from Dormant → Destroyed
- Invalid: Destroy from Destroyed → error
- Invalid: Checkpoint from Dormant → error (only from Active)

### Pane runtime (5 tests)
- Push command block, verify in buffer
- Ring buffer evicts at capacity
- Current block returns most recent
- Empty buffer returns None
- Pane state transitions (Running → Exited)

### Group policy (5 tests)
- Create group, add session
- max_sessions enforcement (can_create_session returns false)
- on_session_empty returns correct policy
- on_oom returns correct policy
- Remove session from group

### Input authority (5 tests)
- First attacher gets input
- Claim transfers to new client
- Detach holder transfers to next
- All detached → no holder
- Observe client cannot claim Exclusive (or can they? — let claim override)
