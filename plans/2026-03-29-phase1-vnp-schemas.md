# Phase 1: VNP Schema Files — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write all `.vexil` schema files defining the VNP protocol — 62 messages, 20 enums, 7 unions, 4 newtypes, 1 flags type — plus the VEXIL_GAPS.md document.

**Architecture:** Schemas live in `orix/malt/schemas/`, one file per domain. All domain schemas import shared types from `common.vexil`. Persistence schemas import from common. The Vexil compiler at `orix/vexil-lang/` compiles these. Spec reference: `malt/specs/phase1-vnp-schema-design.md`.

**Tech Stack:** Vexil schema language (`.vexil` files), vexilc compiler (`orix/vexil-lang/`)

---

## File Structure

```
orix/malt/
  schemas/
    common.vexil
    envelope.vexil
    handshake.vexil
    shell.vexil
    input.vexil
    mux.vexil
    session.vexil
    task.vexil
    render.vexil
    frame_element.vexil
    system.vexil
    elevate.vexil
    persist/
      session.vexil
      daemon.vexil
      layout.vexil
  VEXIL_GAPS.md
```

---

## Task 1: Shared Types — `schemas/common.vexil`

**Files:**
- Create: `orix/malt/schemas/common.vexil`

This is the foundation — every other schema imports from it. Contains all newtypes, shared enums, flags, and compound types.

- [ ] **Step 1: Create the schemas directory**

```bash
mkdir -p orix/malt/schemas/persist
```

- [ ] **Step 2: Write common.vexil**

Create `orix/malt/schemas/common.vexil`:

```vexil
@version("0.1.0")
namespace malt.common

# ─── Newtypes (ID types) ───

@doc("Content pane identifier.")
newtype PaneId : u32

@doc("Session identifier. 0 = daemon-global (not scoped to a session).")
newtype SessionId : u32

@doc("Layout structure identifier. Every non-leaf node has a stable SplitId.")
newtype SplitId : u32

@doc("Session group identifier.")
newtype GroupId : u32

# ─── Enums ───

@non_exhaustive
@doc("Kind of content hosted in a pane.")
enum PaneKind {
    Shell  @0
    App    @1
    Compat @2
}

@doc("Isolation tier for session processes. Fixed set — platform capabilities are finite.")
enum IsolationTier {
    Bare       @0
    Restricted @1
    Capped     @2
    Contained  @3
}

@doc("Split direction for layout operations.")
enum Direction {
    Horizontal @0
    Vertical   @1
}

@doc("Directional focus navigation.")
enum FocusDir {
    Up   @0
    Down @1
    Left @2
    Right @3
    Next  @4
    Prev  @5
}

@non_exhaustive
@doc("Client color rendering depth.")
enum ColorDepth {
    None     @0
    Basic256 @1
    TrueColor @2
}

@non_exhaustive
@doc("Client Unicode rendering support.")
enum UnicodeLevel {
    None  @0
    Basic @1
    Full  @2
}

@non_exhaustive
@doc("Client inline image protocol support.")
enum ImageProtocol {
    None          @0
    Sixel         @1
    KittyGraphics @2
    ItermInline   @3
}

@non_exhaustive
@doc("Log/diagnostic severity level.")
enum Severity {
    Debug @0
    Info  @1
    Warn  @2
    Error @3
}

@non_exhaustive
@doc("Signal types delivered through VNP messages.")
enum SignalKind {
    Int  @0
    Tstp @1
    Quit @2
    Term @3
    Hup  @4
    Usr1 @5
    Usr2 @6
}

@non_exhaustive
@doc("Mouse button identifier.")
enum MouseButtonKind {
    Left     @0
    Right    @1
    Middle   @2
    ScrollUp @3
    ScrollDown @4
}

@non_exhaustive
@doc("Mouse event type.")
enum MouseEventKind {
    Press   @0
    Release @1
    Move    @2
}

@non_exhaustive
@doc("Pane process lifecycle state.")
enum PaneState {
    Running @0
    Stopped @1
    Exited  @2
    Error   @3
}

@non_exhaustive
@doc("Session lifecycle state per architecture §13.")
enum SessionState {
    Active     @0
    Dormant    @1
    Checkpoint @2
    Destroyed  @3
}

@non_exhaustive
@doc("Multi-client input authority mode. Architecture spec notes future cooperative sharing extension.")
enum InputAuthority {
    Exclusive @0
    Shared    @1
    Observe   @2
}

@non_exhaustive
@doc("Group policy: action when last session detaches.")
enum OnEmpty {
    Destroy    @0
    Keep       @1
    Checkpoint @2
}

@non_exhaustive
@doc("Group policy: OOM response strategy.")
enum OnOom {
    KillOffender       @0
    CheckpointThenKill @1
    PauseAndNotify     @2
}

# ─── Flags ───

@doc("Keyboard modifier flags. 4-bit, sub-byte packed.")
flags KeyModifiers {
    Shift @0
    Ctrl  @1
    Alt   @2
    Meta  @3
}

# ─── Unions ───

@non_exhaustive
@doc("Split size constraint for layout children.")
union SplitSize {
    Ratio @0 { value @0 : f32 }
    Fixed @1 { value @0 : u16 }
    Min   @2 { value @0 : u16 }
    Max   @3 { value @0 : u16 }
}

@non_exhaustive
@doc("Abstract layout tree. Recursive — Split and Tabbed contain child LayoutNodes.")
union LayoutNode {
    Leaf   @0 { pane_id @0 : PaneId }
    Split  @1 {
        split_id  @0 : SplitId
        direction @1 : Direction
        sizes     @2 : array<SplitSize>
        children  @3 : array<LayoutNode>
    }
    Tabbed @2 {
        split_id @0 : SplitId
        active   @1 : u16
        children @2 : array<LayoutNode>
    }
    Float  @3 {
        pane_id @0 : PaneId
        x       @1 : u16
        y       @2 : u16
        width   @3 : u16
        height  @4 : u16
    }
}

# ─── Messages (shared compound types) ───

@doc("Tab metadata for panes that are members of a tabbed layout.")
message TabContext {
    label     @0 : string
    is_active @1 : bool
    tab_index @2 : u16
}

@doc("Resolved pane with absolute screen coordinates.")
message ResolvedPane {
    pane_id     @0 : PaneId
    x           @1 : u16
    y           @2 : u16
    width       @3 : u16
    height      @4 : u16
    focused     @5 : bool
    visible     @6 : bool
    z_order     @7 : u32
    tab_context @8 : optional<TabContext>
}

@doc("Fully resolved text style. All theme tokens eliminated, colors resolved to RGB.")
message ResolvedStyle {
    fg            @0 : rgb
    bg            @1 : rgb
    bold          @2 : bool
    italic        @3 : bool
    underline     @4 : bool
    dim           @5 : bool
    strikethrough @6 : bool
    reverse       @7 : bool
    blink         @8 : bool
}

@doc("Client capability declaration for capability-based rendering adaptation.")
message ClientCapabilities {
    color_depth    @0 : ColorDepth
    unicode        @1 : UnicodeLevel
    image_protocol @2 : ImageProtocol
    overlay        @3 : bool
    vt_passthrough @4 : bool
    max_fps        @5 : u16
}

@doc("Summary info for a session, used in ListSessions response and HelloAck.")
message SessionInfo {
    session_id @0 : SessionId
    name       @1 : optional<string>
    pane_count @2 @varint : u16
    isolation  @3 : IsolationTier
    state      @4 : SessionState
}

@doc("Per-session theme color overrides.")
message ThemeOverride {
    fg        @0 : optional<rgb>
    bg        @1 : optional<rgb>
    cursor    @2 : optional<rgb>
    selection @3 : optional<rgb>
}
```

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add schemas/common.vexil
git commit -m "schema: add common.vexil — shared types for VNP protocol"
```

---

## Task 2: Envelope — `schemas/envelope.vexil`

**Files:**
- Create: `orix/malt/schemas/envelope.vexil`

- [ ] **Step 1: Write envelope.vexil**

Create `orix/malt/schemas/envelope.vexil`:

```vexil
@version("0.1.0")
namespace malt.envelope

@doc("VNP message envelope. Bit-packed header preceding every payload.")
message Envelope {
    @doc("Wire format version (0-15). Distinct from Hello.version which is the software protocol version.")
    wire_version @0 : u4

    @doc("Message domain (0-15). See domain ID assignment table in schema design spec.")
    domain       @1 : u4

    @doc("Message type within domain (0-127).")
    msg_type     @2 : u7

    @doc("Target session. 0 = daemon-global.")
    session_id   @3 : u32

    @doc("Microseconds since daemon start. Wraps at ~8.9 years.")
    timestamp    @4 : u48

    @doc("Optional correlation ID for request/response pairing.")
    msg_id       @5 : optional<u32>
}
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add schemas/envelope.vexil
git commit -m "schema: add envelope.vexil — VNP bit-packed message header"
```

---

## Task 3: Handshake + Shell — `schemas/handshake.vexil`, `schemas/shell.vexil`

**Files:**
- Create: `orix/malt/schemas/handshake.vexil`
- Create: `orix/malt/schemas/shell.vexil`

- [ ] **Step 1: Write handshake.vexil**

Create `orix/malt/schemas/handshake.vexil`:

```vexil
@version("0.1.0")
namespace malt.handshake

import malt.common

# Priority: Reliable
@doc("Client → daemon: initiate connection. First message on any transport.")
@domain(Handshake) @type(0x01) @revision(1)
message Hello {
    @doc("Software protocol version supported by this client.")
    version      @0 : u32
    client_type  @1 : string
    capabilities @2 : ClientCapabilities
}

# Priority: Reliable
@doc("Daemon → client: connection accepted with negotiated parameters.")
@domain(Handshake) @type(0x02) @revision(1)
message HelloAck {
    negotiated_version @0 : u32
    sessions           @1 : array<SessionInfo>
    @doc("Wall-clock offset for converting envelope timestamps to absolute time.")
    start_time_offset  @2 : i64
}

# Priority: Reliable
@doc("Daemon → client: version mismatch, connection will be terminated.")
@domain(Handshake) @type(0x03) @revision(1)
message VersionSkew {
    expected_min    @0 : u32
    expected_max    @1 : u32
    client_version  @2 : u32
    reason          @3 : string
}
```

- [ ] **Step 2: Write shell.vexil**

Create `orix/malt/schemas/shell.vexil`:

```vexil
@version("0.1.0")
namespace malt.shell

import malt.common

# Priority: Reliable
@doc("A command has started executing in a shell pane.")
@domain(Shell) @type(0x01) @revision(1)
message CommandStarted {
    @doc("Monotonic command ID for correlating with CommandFinished and OutputChunk.")
    command_id @0 : u32
    cmd        @1 : string
}

# Priority: Reliable
@doc("A command has finished executing.")
@domain(Shell) @type(0x02) @revision(1)
message CommandFinished {
    command_id  @0 : u32
    exit_code   @1 : i32
    duration_us @2 : u64
}

# Priority: Reliable
@doc("Shell is ready for the next command. Emitted after prompt is displayed.")
@domain(Shell) @type(0x03) @revision(1)
message PromptReady {
    cwd @0 : string
}

# Priority: Normal
@doc("Raw output data from a command or PTY.")
@doc("command_tag is present when output originates from a known command")
@doc("(MASH sets it at emission time); absent for startup output, background")
@doc("jobs with no command association, or raw PTY output from compat panes.")
@domain(Shell) @type(0x04) @revision(1)
message OutputChunk {
    data        @0 : bytes
    command_tag @1 : optional<string>
}
```

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add schemas/handshake.vexil schemas/shell.vexil
git commit -m "schema: add handshake and shell domain schemas"
```

---

## Task 4: Input — `schemas/input.vexil`

**Files:**
- Create: `orix/malt/schemas/input.vexil`

- [ ] **Step 1: Write input.vexil**

Create `orix/malt/schemas/input.vexil`:

```vexil
@version("0.1.0")
namespace malt.input

import malt.common

@non_exhaustive
@doc("Named keyboard keys (non-printable).")
enum NamedKey {
    Enter     @0
    Escape    @1
    Tab       @2
    Backspace @3
    Insert    @4
    Delete    @5
    Home      @6
    End       @7
    PageUp    @8
    PageDown  @9
    Up        @10
    Down      @11
    Left      @12
    Right     @13
}

@non_exhaustive
@doc("Key value — Unicode character, named key, or function key.")
union KeyValue {
    Char     @0 { codepoint @0 : u32 }
    Named    @1 { key @0 : NamedKey }
    Function @2 { number @0 : u8 }
}

# Priority: Critical (not on bus — direct routing)
@doc("Keyboard input event. Routed directly from daemon core to focused pane owner.")
@domain(Input) @type(0x01) @revision(1)
message KeyEvent {
    key       @0 : KeyValue
    modifiers @1 : KeyModifiers
}

# Priority: Critical
@doc("Mouse input event.")
@domain(Input) @type(0x02) @revision(1)
message MouseEvent {
    x         @0 : u16
    y         @1 : u16
    kind      @2 : MouseEventKind
    button    @3 : MouseButtonKind
    modifiers @4 : KeyModifiers
}

# Priority: Critical
@doc("Signal delivery via VNP. Daemon translates Ctrl-C → SignalInput{Int}, etc.")
@domain(Input) @type(0x03) @revision(1)
message SignalInput {
    signal @0 : SignalKind
}

# Priority: Critical
@doc("Terminal resize event. Daemon enforces PTY resize before emitting RenderCommands.")
@domain(Input) @type(0x04) @revision(1)
message Resize {
    cols @0 : u16
    rows @1 : u16
}
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add schemas/input.vexil
git commit -m "schema: add input domain — KeyEvent, MouseEvent, SignalInput, Resize"
```

---

## Task 5: Mux + Session — `schemas/mux.vexil`, `schemas/session.vexil`

**Files:**
- Create: `orix/malt/schemas/mux.vexil`
- Create: `orix/malt/schemas/session.vexil`

- [ ] **Step 1: Write mux.vexil**

Create `orix/malt/schemas/mux.vexil`:

```vexil
@version("0.1.0")
namespace malt.mux

import malt.common

# All mux messages are Priority: Reliable

@doc("A new pane has been created.")
@domain(Mux) @type(0x01) @revision(1)
message PaneCreated {
    pane_id @0 : PaneId
    kind    @1 : PaneKind
    title   @2 : optional<string>
}

@doc("A pane has been destroyed.")
@domain(Mux) @type(0x02) @revision(1)
message PaneDestroyed {
    pane_id @0 : PaneId
}

@doc("The layout tree has changed. Sent after any structural operation.")
@domain(Mux) @type(0x03) @revision(1)
message LayoutChanged {
    layout @0 : LayoutNode
}

@doc("Split an existing pane, creating a new sibling.")
@domain(Mux) @type(0x04) @revision(1)
message SplitPane {
    target    @0 : PaneId
    direction @1 : Direction
    size      @2 : SplitSize
    @doc("Kind of pane to create. Defaults to Shell when absent.")
    kind      @3 : optional<PaneKind>
}

@doc("Close a pane and remove it from the layout.")
@domain(Mux) @type(0x05) @revision(1)
message ClosePane {
    pane_id @0 : PaneId
}

@doc("Convert a pane to a floating overlay.")
@domain(Mux) @type(0x06) @revision(1)
message FloatPane {
    pane_id @0 : PaneId
    x       @1 : u16
    y       @2 : u16
    width   @3 : u16
    height  @4 : u16
}

@doc("Swap the positions of two panes in the layout.")
@domain(Mux) @type(0x07) @revision(1)
message SwapPanes {
    a @0 : PaneId
    b @1 : PaneId
}

@doc("Move focus in a direction.")
@domain(Mux) @type(0x08) @revision(1)
message FocusDirection {
    direction @0 : FocusDir
}

@doc("Resize a specific child within a split node.")
@domain(Mux) @type(0x09) @revision(1)
message ResizeSplit {
    split_id    @0 : SplitId
    child_index @1 : u16
    size        @2 : SplitSize
}

@doc("Save the current layout as a named preset.")
@domain(Mux) @type(0x0A) @revision(1)
message SaveLayout {
    name @0 : string
}

@doc("Load a named layout preset, replacing the current layout.")
@domain(Mux) @type(0x0B) @revision(1)
message LoadLayout {
    name @0 : string
}
```

- [ ] **Step 2: Write session.vexil**

Create `orix/malt/schemas/session.vexil`:

```vexil
@version("0.1.0")
namespace malt.session

import malt.common

# All session messages are Priority: Reliable

@doc("Create a new session.")
@domain(Session) @type(0x01) @revision(1)
message CreateSession {
    name      @0 : optional<string>
    isolation @1 : IsolationTier
    group     @2 : optional<GroupId>
}

@doc("Attach to an existing session.")
@domain(Session) @type(0x02) @revision(1)
message AttachSession {
    session_id @0 : SessionId
    authority  @1 : InputAuthority
}

@doc("Detach from a session.")
@domain(Session) @type(0x03) @revision(1)
message DetachSession {
    session_id @0 : SessionId
}

@doc("Request the list of available sessions.")
@domain(Session) @type(0x04) @revision(1)
message ListSessions {}

@doc("Response to ListSessions.")
@domain(Session) @type(0x05) @revision(1)
message SessionList {
    sessions @0 : array<SessionInfo>
}

@doc("Request to claim or change input authority on a session.")
@domain(Session) @type(0x06) @revision(1)
message InputClaim {
    session_id @0 : SessionId
    authority  @1 : InputAuthority
}

@doc("Notification that input authority has changed.")
@domain(Session) @type(0x07) @revision(1)
message InputAuthorityChanged {
    session_id @0 : SessionId
    holder     @1 : optional<string>
    authority  @2 : InputAuthority
}
```

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add schemas/mux.vexil schemas/session.vexil
git commit -m "schema: add mux and session domains — layout ops, multi-client authority"
```

---

## Task 6: Task + System — `schemas/task.vexil`, `schemas/system.vexil`

**Files:**
- Create: `orix/malt/schemas/task.vexil`
- Create: `orix/malt/schemas/system.vexil`

- [ ] **Step 1: Write task.vexil**

Create `orix/malt/schemas/task.vexil`:

```vexil
@version("0.1.0")
namespace malt.task

import malt.common

@non_exhaustive
@doc("Task lifecycle state.")
enum TaskState {
    Pending  @0
    Running  @1
    Complete @2
    Failed   @3
}

# All task messages are Priority: Reliable

@doc("Create a new background task.")
@domain(Task) @type(0x01) @revision(1)
message TaskCreate {
    kind     @0 : string
    metadata @1 : optional<string>
}

@doc("Task status update.")
@domain(Task) @type(0x02) @revision(1)
message TaskStatus {
    task_id  @0 : u32
    state    @1 : TaskState
    progress @2 : optional<u8>
}

@doc("Task has completed (success or failure).")
@domain(Task) @type(0x03) @revision(1)
message TaskComplete {
    task_id @0 : u32
    result  @1 : result<string, string>
}
```

- [ ] **Step 2: Write system.vexil**

Create `orix/malt/schemas/system.vexil`:

```vexil
@version("0.1.0")
namespace malt.system

import malt.common

@non_exhaustive
@doc("Plugin lifecycle event type.")
enum PluginEventType {
    Loaded   @0
    Unloaded @1
    Error    @2
    Log      @3
}

# Priority: Reliable
@doc("Structured output from a command, parsed by the Structured Output Parser.")
@domain(System) @type(0x01) @revision(1)
message StructuredOutput {
    @doc("Parser kind, e.g. 'CargoBuild', 'RustcError', 'GitStatus'.")
    kind    @0 : string
    content @1 : bytes
}

# Priority: Low
@doc("Plugin lifecycle or diagnostic event.")
@domain(System) @type(0x02) @revision(1)
message PluginEvent {
    plugin_id  @0 : string
    event_type @1 : PluginEventType
    data       @2 : optional<bytes>
}

# Priority: Low
@doc("Diagnostic message from any subsystem.")
@domain(System) @type(0x03) @revision(1)
message Diagnostic {
    severity @0 : Severity
    message  @1 : string
    source   @2 : optional<string>
}

# Priority: Low
@doc("Liveness heartbeat. Used by compat workers and other subsystems.")
@domain(System) @type(0x04) @revision(1)
message Heartbeat {
    @doc("Monotonic sequence number for detecting missed heartbeats.")
    seq    @0 @varint : u32
    @doc("Identifies the heartbeat source, e.g. 'compat-worker-3'.")
    source @1 : optional<string>
}

# Priority: Reliable
@doc("Recoverable error from a subsystem.")
@domain(System) @type(0x05) @revision(1)
message Error {
    code    @0 @varint : u32
    message @1 : string
    context @2 : optional<string>
}
```

- [ ] **Step 3: Commit**

```bash
cd orix/malt
git add schemas/task.vexil schemas/system.vexil
git commit -m "schema: add task and system domains"
```

---

## Task 7: Render — `schemas/render.vexil`

**Files:**
- Create: `orix/malt/schemas/render.vexil`

The largest domain schema — contains the RenderCommand union (14 variants) and 7 messages.

- [ ] **Step 1: Write render.vexil**

Create `orix/malt/schemas/render.vexil`:

```vexil
@version("0.1.0")
namespace malt.render

import malt.common

@non_exhaustive
@doc("Image pixel data format for DrawImage.")
enum ImageFormat {
    Rgba  @0
    Png   @1
    Sixel @2
}

@non_exhaustive
@doc("Concrete drawing instruction. All positions absolute, all colors resolved to RGB.")
union RenderCommand {
    SetCursor    @0 { x @0 : u16  y @1 : u16 }
    SetClip      @1 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16 }
    ClearClip    @2 {}
    DrawText     @3 {
        x     @0 : u16
        y     @1 : u16
        text  @2 : string
        style @3 : ResolvedStyle
    }
    DrawRect     @4 {
        x      @0 : u16
        y      @1 : u16
        width  @2 : u16
        height @3 : u16
        style  @4 : ResolvedStyle
    }
    DrawBorder   @5 {
        x      @0 : u16
        y      @1 : u16
        width  @2 : u16
        height @3 : u16
        style  @4 : ResolvedStyle
    }
    DrawLine     @6 {
        x1    @0 : u16
        y1    @1 : u16
        x2    @2 : u16
        y2    @3 : u16
        style @4 : ResolvedStyle
    }
    DrawImage    @7 {
        x      @0 : u16
        y      @1 : u16
        width  @2 : u16
        height @3 : u16
        data   @4 : bytes
        format @5 : ImageFormat
    }
    ScrollRegion @8 {
        x      @0 : u16
        y      @1 : u16
        width  @2 : u16
        height @3 : u16
        delta  @4 : i16
    }
    PushLayer    @9 {}
    PopLayer     @10 {}
    @doc("VT passthrough for compat panes. rect defines the pane's screen region.")
    WriteRaw     @11 {
        data   @0 : bytes
        x      @1 : u16
        y      @2 : u16
        width  @3 : u16
        height @4 : u16
    }
    Clear        @12 {}
    Flush        @13 {}
}

# Priority: High
@doc("One frame's worth of drawing instructions.")
@domain(Render) @type(0x01) @revision(1)
message RenderBatch {
    frame_seq @0 : u64
    commands  @1 : array<RenderCommand>
}

# Priority: Normal
@doc("Client acknowledges receipt of a frame. Backpressure mechanism.")
@domain(Render) @type(0x02) @revision(1)
message FrameAck {
    frame_seq @0 : u64
}

# Priority: Reliable
@doc("Atomic snapshot sent on client attach. Includes full layout and initial frame.")
@domain(Render) @type(0x03) @revision(1)
message InitialState {
    frame_seq @0 : u64
    layout    @1 : LayoutNode
    panes     @2 : array<ResolvedPane>
    commands  @3 : array<RenderCommand>
}

# Priority: Reliable
@doc("Client requests a fresh InitialState snapshot.")
@domain(Render) @type(0x04) @revision(1)
message SyncRequest {}

# Priority: Reliable
@doc("Daemon disconnects a client that hasn't sent FrameAck in 10 seconds.")
@domain(Render) @type(0x05) @revision(1)
message SlowClientDisconnect {
    reason @0 : string
}

# Priority: Normal
@doc("Client requests scrollback content for a pane.")
@domain(Render) @type(0x06) @revision(1)
message ScrollbackRequest {
    pane_id @0 : PaneId
    lines   @1 @varint : u32
}

# Priority: Normal
@doc("Scrollback content response.")
@domain(Render) @type(0x07) @revision(1)
message ScrollbackResponse {
    pane_id @0 : PaneId
    data    @1 : bytes
}
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add schemas/render.vexil
git commit -m "schema: add render domain — RenderCommand union (14 variants), frame lifecycle"
```

---

## Task 8: FrameElement — `schemas/frame_element.vexil`

**Files:**
- Create: `orix/malt/schemas/frame_element.vexil`

Internal, unstable. Separate file so it can evolve without touching stable domain schemas.

- [ ] **Step 1: Write frame_element.vexil**

Create `orix/malt/schemas/frame_element.vexil`:

```vexil
@version("0.1.0")
namespace malt.frame_element

import malt.common

@non_exhaustive
@doc("Semantic UI primitive. Internal to daemon — clients never see this.")
@doc("Recursive: Split, Stack, Padded, Centered, Scrollable, Custom contain child elements.")
union FrameElement {
    Text          @0 { text @0 : string  style @1 : ResolvedStyle }
    Paragraph     @1 { lines @0 : array<string>  style @1 : ResolvedStyle }
    Empty         @2 {}
    Split         @3 {
        direction @0 : Direction
        sizes     @1 : array<SplitSize>
        children  @2 : array<FrameElement>
    }
    Stack         @4 { children @0 : array<FrameElement> }
    Padded        @5 {
        top    @0 : u16
        right  @1 : u16
        bottom @2 : u16
        left   @3 : u16
        child  @4 : FrameElement
    }
    Centered      @6 { child @0 : FrameElement }
    Scrollable    @7 { offset @0 : u32  child @1 : FrameElement }
    VtPassthrough @8 { data @0 : bytes }
    Custom        @9 {
        type_id  @0 : string
        data     @1 : bytes
        fallback @2 : optional<FrameElement>
    }
}
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add schemas/frame_element.vexil
git commit -m "schema: add frame_element.vexil — internal unstable UI primitives (10 variants)"
```

---

## Task 9: Elevate — `schemas/elevate.vexil`

**Files:**
- Create: `orix/malt/schemas/elevate.vexil`

Restricted schema for the elevated helper binary. Minimal audit surface.

- [ ] **Step 1: Write elevate.vexil**

Create `orix/malt/schemas/elevate.vexil`:

```vexil
@version("0.1.0")
namespace malt.elevate

import malt.common

@doc("Daemon → helper: initiate authenticated connection.")
message ElevateHello {
    @doc("Single-use nonce from the nonce file. Rotated hourly with 30s overlap.")
    nonce   @0 : u64
    version @1 : u32
}

@doc("Helper → daemon: authentication result.")
message ElevateHelloAck {
    nonce    @0 : u64
    accepted @1 : bool
    reason   @2 : optional<string>
}

@doc("Wrapper carrying correlation ID for request/response pairing.")
message ElevateRequestEnvelope {
    request_id @0 : u32
    request    @1 : ElevateRequest
}

@non_exhaustive
@doc("Privileged operation request. Platform-specific variants.")
union ElevateRequest {
    # Linux
    CreateNamespace  @0 { pid @0 : u32  tier @1 : IsolationTier }
    MountOverlay     @1 { lower @0 : string  upper @1 : string  merged @2 : string }
    SetCgroup        @2 { pid @0 : u32  memory_mb @1 : u32  cpu_pct @2 : u16 }
    SetupNetns       @3 { pid @0 : u32  bridge @1 : string  veth_host @2 : string  veth_ns @3 : string }
    ApplySeccomp     @4 { pid @0 : u32  policy @1 : bytes }
    # Windows
    CreateSymlink    @5 { target @0 : string  link @1 : string }
    CreateRestrictedToken @6 { pid @0 : u32  tier @1 : IsolationTier }
    ManageHcsContainer @7 { operation @0 : string  config @1 : bytes }
    # macOS
    ApplySeatbelt    @8 { pid @0 : u32  profile @1 : string }
    # Cross-platform
    BindPort         @9 { port @0 : u16  socket_path @1 : string }
}

@doc("Helper → daemon: operation result.")
message ElevateResponse {
    request_id @0 : u32
    result     @1 : result<bytes, string>
}

@doc("Either side: graceful shutdown.")
message ElevateShutdown {
    reason @0 : string
}
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add schemas/elevate.vexil
git commit -m "schema: add elevate.vexil — restricted schema for privileged helper (10 operation variants)"
```

---

## Task 10: Persistence — `schemas/persist/*.vexil`

**Files:**
- Create: `orix/malt/schemas/persist/session.vexil`
- Create: `orix/malt/schemas/persist/daemon.vexil`
- Create: `orix/malt/schemas/persist/layout.vexil`

- [ ] **Step 1: Write persist/session.vexil**

Create `orix/malt/schemas/persist/session.vexil`:

```vexil
@version("0.1.0")
namespace malt.persist.session

import malt.common

@doc("Persisted session state. Stored in .vx (text) and optionally .vxb (binary).")
message PersistedSession {
    @doc("Persistence format version. Starts at 1. Read first for version-gating.")
    schema_version @0 : u32
    id             @1 : SessionId
    name           @2 : optional<string>
    layout         @3 : LayoutNode
    focus          @4 : PaneId
    @doc("Key is PaneId as raw u32. See VEXIL_GAPS.md — newtypes cannot be map keys.")
    panes          @5 : map<u32, PersistedPane>
    theme          @6 : optional<ThemeOverride>
    group          @7 : optional<GroupId>
    isolation      @8 : IsolationTier
}

@doc("Persisted pane state within a session.")
message PersistedPane {
    cwd       @0 : string
    title     @1 : optional<string>
    pane_type @2 : PersistedPaneType
}

@non_exhaustive
@doc("Discriminated pane type with type-specific restoration data.")
union PersistedPaneType {
    Shell  @0 { shell_path @0 : string }
    App    @1 { app_id @0 : string  config @1 : optional<bytes> }
    Compat @2 { program @0 : string  args @1 : array<string> }
}
```

- [ ] **Step 2: Write persist/daemon.vexil**

Create `orix/malt/schemas/persist/daemon.vexil`:

```vexil
@version("0.1.0")
namespace malt.persist.daemon

import malt.common

@doc("Group policy governing resource limits and lifecycle behavior.")
message GroupPolicy {
    min_tier          @0 : IsolationTier
    max_memory_mb     @1 @varint : u32
    max_cpu_cores     @2 : u16
    max_sessions      @3 @varint : u16
    ttl_secs          @4 : optional<u32>
    idle_timeout_secs @5 : optional<u32>
    on_empty          @6 : OnEmpty
    on_oom            @7 : OnOom
}

@doc("Persisted state for a session group.")
message GroupState {
    id          @0 : GroupId
    name        @1 : string
    policy      @2 : optional<GroupPolicy>
    session_ids @3 : array<SessionId>
}

@doc("Top-level daemon state file. Stored in daemon.vx.")
message DaemonState {
    @doc("Persistence format version. Starts at 1. Read first for version-gating.")
    schema_version  @0 : u32
    sessions        @1 : array<SessionId>
    active_groups   @2 : array<GroupState>
    next_session_id @3 : u32
    next_pane_id    @4 : u32
}
```

- [ ] **Step 3: Write persist/layout.vexil**

Create `orix/malt/schemas/persist/layout.vexil`:

```vexil
@version("0.1.0")
namespace malt.persist.layout

import malt.common

@doc("Named layout preset for SaveLayout/LoadLayout.")
message PersistedLayout {
    name   @0 : string
    layout @1 : LayoutNode
}
```

- [ ] **Step 4: Commit**

```bash
cd orix/malt
git add schemas/persist/
git commit -m "schema: add persistence schemas — session, daemon state, layout presets"
```

---

## Task 11: Vexil Gaps Document

**Files:**
- Create: `orix/malt/VEXIL_GAPS.md`

- [ ] **Step 1: Write VEXIL_GAPS.md**

Create `orix/malt/VEXIL_GAPS.md`:

```markdown
# Vexil Language Gaps

Shortcomings in `vexil-lang` discovered during VNP schema design. Each gap includes the problem, its impact on MALT schemas, the current workaround, and the requested fix for the vexil-lang project.

These are tracked here (not as vexil-lang issues) because they were discovered during MALT design. They should be filed as vexil-lang issues when implementation begins.

---

## Gap 1: Map keys don't accept newtypes

**Problem:** `map<PaneId, PersistedPane>` is invalid because the Vexil spec restricts map key types to primitives, string, bytes, uuid, enum, and flags. Newtypes are excluded even when they wrap a valid key type.

**Impact:** Persistence schemas use `map<u32, PersistedPane>` instead of `map<PaneId, PersistedPane>`, losing type safety on map keys. Generated Rust code uses raw `u32` keys.

**Workaround:** Use the raw primitive type for map keys. Document the intended key type with `@doc`.

**Requested fix:** Newtypes wrapping valid map key types should themselves be valid map key types. The wire encoding is identical to the inner type — this is purely a type-checker restriction.

---

## Gap 2: No custom/user-defined annotation support

**Problem:** Cannot attach arbitrary key-value metadata to declarations. VNP needs to associate priority class, routing hints, and other protocol metadata with message types.

**Impact:** Priority class (Critical, Reliable, High, Normal, Low) must be maintained as a separate Rust const table in `malt-protocol` rather than co-located with the schema definition.

**Workaround:** Document priority via `@doc("Priority: Critical")` on each message and maintain a `priority_of()` mapping function in `malt-protocol` Rust code.

**Requested fix:** Support user-defined annotations — e.g., `@meta(key, value)` or a pluggable annotation registry. Codegen backends should emit custom annotations as associated constants or a queryable metadata map.

---

## Gap 3: Potential `None` variant conflict in Rust codegen

**Problem:** Enum variants named `None` (e.g., `ColorDepth::None`) may conflict with `Option::None` in generated Rust code, depending on codegen output and usage context.

**Impact:** Several VNP enums use `None` as a variant name (`ColorDepth`, `UnicodeLevel`, `ImageProtocol`).

**Workaround:** Verify codegen output during Phase 1 sub-project 2 (malt-protocol crate). If conflicts arise, rename variants to `Off` or `Unsupported`.

**Requested fix:** Rust codegen should handle `None` variant names safely — either by always qualifying enum variant usage or by emitting a warning when a variant name collides with Rust built-in names.

---

## Gap 4: No standard library types

**Problem:** Common semantic types like `Duration`, `IpAddr`, `SemVer`, `Url` are listed as future `vexil.std` additions but not available today.

**Impact:** VNP schemas use raw primitives with `@doc` annotations explaining semantics — `u64` for microsecond durations, `string` for version strings, `i64` for timestamps. Type safety and self-documentation suffer.

**Workaround:** Use raw types with `@doc` annotations.

**Requested fix:** Implement `vexil.std` with at least: `Duration` (i64 microseconds), `Timestamp` (i64 microseconds since epoch), `SemVer` (string with validation).
```

- [ ] **Step 2: Commit**

```bash
cd orix/malt
git add VEXIL_GAPS.md
git commit -m "docs: add VEXIL_GAPS.md — 4 vexil-lang shortcomings found during schema design"
```

---

## Task 12: Compilation Verification

**Files:**
- No new files — verification task

Attempt to compile all schemas with vexilc to validate syntax.

- [ ] **Step 1: Check if vexilc can compile standalone files**

Run: `cd orix/vexil-lang && cargo run -p vexilc -- --help`

Check what arguments vexilc accepts. We need to know: can it take a directory of `.vexil` files with imports between them?

- [ ] **Step 2: Attempt to compile common.vexil**

Run: `cd orix/vexil-lang && cargo run -p vexilc -- compile ../malt/schemas/common.vexil`

If this fails, note the error. Common has no imports, so it should compile if the syntax is valid. If vexilc requires different invocation, adjust.

- [ ] **Step 3: Attempt to compile a domain schema with imports**

Run: `cd orix/vexil-lang && cargo run -p vexilc -- compile ../malt/schemas/shell.vexil --include ../malt/schemas/`

This tests cross-file imports. If vexilc doesn't support `--include`, note it and try alternative approaches (e.g., setting a schema search path).

- [ ] **Step 4: If compilation works, compile all schemas**

Run: `cd orix/vexil-lang && for f in ../malt/schemas/*.vexil ../malt/schemas/persist/*.vexil; do echo "=== $f ===" && cargo run -p vexilc -- compile "$f" --include ../malt/schemas/ || echo "FAIL: $f"; done`

Expected: All 15 `.vexil` files compile. Record any failures.

- [ ] **Step 5: If compilation fails, document the gap**

If vexilc cannot compile the schemas (e.g., missing import path support, missing annotation support, syntax errors), document:
- Which files fail and why
- Whether the failure is a vexilc limitation or a schema syntax error
- Fix syntax errors in-place; add vexilc limitations to `VEXIL_GAPS.md`

- [ ] **Step 6: Commit any fixes**

```bash
cd orix/malt
git add schemas/ VEXIL_GAPS.md
git commit -m "fix: resolve schema compilation issues found during verification"
```

---

## Verification

After all tasks are done, confirm:

1. All 15 `.vexil` files exist in `schemas/` and `schemas/persist/`
2. `VEXIL_GAPS.md` exists with 4 documented gaps
3. Every type referenced in a field is defined (either in the same file or in `common.vexil`)
4. Ordinals are sequential within each message/union (no gaps, no duplicates)
5. All `@non_exhaustive` annotations match the spec conventions
6. Schema compilation passes (or failures are documented)
