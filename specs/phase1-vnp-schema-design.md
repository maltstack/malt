# Phase 1 Sub-Project 1: VNP Schema Design

## Goal

Define all `.vexil` schema files for the VNP protocol — message types across 8 domains, shared types, persistence schemas, envelope, and the elevate restricted schema. These schemas are the vocabulary that every MALT component speaks.

## Architecture

Schemas live in `orix/malt/schemas/` organized as one file per domain plus shared types, persistence, and elevate. All schemas use the `malt.*` namespace. The Vexil compiler (`orix/vexil-lang/`) compiles these into Rust types via `vexil-codegen-rust`. The `malt-protocol` crate (Phase 1 sub-project 2) consumes the generated code.

## Spec Reference

`malt/specs/architecture.md` §6 (VNP), §7 (Rendering Types), §4 (Layout), §5 (Shell), §10 (Isolation), §13 (Persistence).

---

## File Organization

```
orix/malt/
  schemas/
    common.vexil           # Newtypes, shared enums, flags, compound types
    envelope.vexil         # Envelope message (sub-byte packed header)
    handshake.vexil        # Hello, HelloAck, VersionSkew
    shell.vexil            # CommandStarted, CommandFinished, PromptReady, OutputChunk
    input.vexil            # KeyEvent, MouseEvent, SignalInput, Resize
    mux.vexil              # PaneCreated/Destroyed, LayoutChanged, SplitPane, ClosePane, etc.
    session.vexil          # CreateSession, AttachSession, DetachSession, ListSessions, SessionList
    task.vexil             # TaskCreate, TaskStatus, TaskComplete
    render.vexil           # RenderCommand union, RenderBatch, FrameAck, InitialState, etc.
    frame_element.vexil    # FrameElement union (internal, unstable)
    system.vexil           # StructuredOutput, PluginEvent, Diagnostic, Heartbeat, Error
    elevate.vexil          # ElevateHello/Ack, ElevateRequest, ElevateResponse, Shutdown
    persist/
      session.vexil        # PersistedSession, PersistedPane, PersistedPaneType
      daemon.vexil         # DaemonState, GroupState
      layout.vexil         # PersistedLayout (named presets)
  VEXIL_GAPS.md            # Documented vexil-lang shortcomings
```

---

## Schema Conventions

1. **Namespace**: `malt.{domain}` — e.g., `malt.shell`, `malt.input`, `malt.common`, `malt.persist.session`, `malt.elevate`.

2. **Versioning**: All schemas start at `@version("0.1.0")`. Wire-breaking changes bump minor until 1.0.

3. **Ordinals**: Start at `@0`, sequential, no gaps in initial design. Gaps only appear later via `@removed` tombstones.

4. **Wire annotations**: `@varint` on large integer fields that are typically small (exit codes, counts). `@delta` on sequential monotonic timestamps.

5. **Non-exhaustive**: All enums and unions that clients or plugins interact with get `@non_exhaustive`. Only internal enums with a truly fixed variant set (e.g., `Direction`) are exhaustive.

6. **Documentation**: Every message and field gets `@doc(...)`. Becomes generated Rust doc comments and feeds into the VNP protocol spec document.

7. **Priority metadata**: Documented via `@doc("Priority: Critical")` on messages. Mapped to a Rust const table in `malt-protocol`. Tracked in `VEXIL_GAPS.md` as a workaround for missing custom annotation support.

8. **Envelope annotations**: Messages use `@domain(Shell)`, `@type(0x01)`, `@revision(1)` for wire dispatch.

---

## Schema Definitions

### `schemas/common.vexil`

**Namespace:** `malt.common`

**Newtypes (ID types):**

| Name | Inner | Purpose |
|------|-------|---------|
| `PaneId` | `u32` | Content pane identifier |
| `SessionId` | `u32` | Session identifier (0 = daemon-global) |
| `SplitId` | `u32` | Layout structure identifier |
| `GroupId` | `u32` | Session group identifier |

**Enums:**

| Name | Variants | Notes |
|------|----------|-------|
| `PaneKind` | Shell @0, App @1, Compat @2 | `@non_exhaustive` |
| `IsolationTier` | Bare @0, Restricted @1, Capped @2, Contained @3 | Exhaustive (fixed set) |
| `Direction` | Horizontal @0, Vertical @1 | Exhaustive |
| `FocusDir` | Up @0, Down @1, Left @2, Right @3, Next @4, Prev @5 | Exhaustive |
| `ColorDepth` | None @0, Basic256 @1, TrueColor @2 | `@non_exhaustive` |
| `UnicodeLevel` | None @0, Basic @1, Full @2 | `@non_exhaustive` |
| `ImageProtocol` | None @0, Sixel @1, KittyGraphics @2, ItermInline @3 | `@non_exhaustive` |
| `Severity` | Debug @0, Info @1, Warn @2, Error @3 | `@non_exhaustive` |
| `SignalKind` | Int @0, Tstp @1, Quit @2, Term @3, Hup @4, Usr1 @5, Usr2 @6 | `@non_exhaustive` |
| `MouseButtonKind` | Left @0, Right @1, Middle @2, ScrollUp @3, ScrollDown @4 | `@non_exhaustive` |
| `MouseEventKind` | Press @0, Release @1, Move @2 | `@non_exhaustive` |

**Flags:**

| Name | Bits | Notes |
|------|------|-------|
| `KeyModifiers` | Shift @0, Ctrl @1, Alt @2, Meta @3 | 4-bit, sub-byte packed |

**Compound types (unions):**

```
union SplitSize {
    Ratio @0 { value @0 : f32 }
    Fixed @1 { value @0 : u16 }
    Min   @2 { value @0 : u16 }
    Max   @3 { value @0 : u16 }
}
```

```
@non_exhaustive
union LayoutNode {
    Leaf   @0 { pane_id @0 : PaneId }
    Split  @1 { direction @0 : Direction
                sizes @1 : array<SplitSize>
                children @2 : array<LayoutNode> }
    Tabbed @2 { active @0 : u16
                children @1 : array<LayoutNode> }
    Float  @3 { pane_id @0 : PaneId
                x @1 : u16  y @2 : u16
                width @3 : u16  height @4 : u16 }
}
```

Note: `LayoutNode` is recursive (`Split.children` and `Tabbed.children` contain `array<LayoutNode>`). Vexil supports this — Rust codegen emits `Box<LayoutNode>`.

**Compound types (messages):**

```
message ResolvedPane {
    pane_id  @0 : PaneId
    x        @1 : u16
    y        @2 : u16
    width    @3 : u16
    height   @4 : u16
    focused  @5 : bool
    visible  @6 : bool
    z_order  @7 : u32
}
```

```
message ResolvedStyle {
    fg        @0 : rgb
    bg        @1 : rgb
    bold      @2 : bool
    italic    @3 : bool
    underline @4 : bool
    dim       @5 : bool
}
```

```
message ClientCapabilities {
    color_depth    @0 : ColorDepth
    unicode        @1 : UnicodeLevel
    image_protocol @2 : ImageProtocol
    overlay        @3 : bool
    vt_passthrough @4 : bool
    max_fps        @5 : u16
}
```

```
message SessionInfo {
    session_id  @0 : SessionId
    name        @1 : optional<string>
    pane_count  @2 : @varint u16
    isolation   @3 : IsolationTier
}
```

```
message ThemeOverride {
    fg        @0 : optional<rgb>
    bg        @1 : optional<rgb>
    cursor    @2 : optional<rgb>
    selection @3 : optional<rgb>
}
```

---

### `schemas/envelope.vexil`

**Namespace:** `malt.envelope`

```
message Envelope {
    version    @0 : u4
    domain     @1 : u4
    msg_type   @2 : u7
    session_id @3 : u32
    timestamp  @4 : u48
    msg_id     @5 : optional<u32>
}
```

Sub-byte fields (`u4`, `u4`, `u7`) pack continuously LSB-first. The `u48` timestamp encodes microseconds since daemon start. The `optional<u32>` msg_id adds a 1-bit presence flag.

**Verification requirement:** A golden-byte test must assert that the generated encoding matches the exact bit layout specified in architecture.md §6. If Vexil's flush rules cause a mismatch (e.g., unexpected byte-boundary flush between sub-byte fields), document as a vexil-lang gap.

---

### `schemas/handshake.vexil`

**Namespace:** `malt.handshake`

| Message | Fields | Priority |
|---------|--------|----------|
| `Hello` | version @0 : u32, client_type @1 : string, capabilities @2 : ClientCapabilities | Reliable |
| `HelloAck` | negotiated_version @0 : u32, sessions @1 : array\<SessionInfo\>, start_time_offset @2 : i64 | Reliable |
| `VersionSkew` | expected_min @0 : u32, expected_max @1 : u32, client_version @2 : u32, reason @3 : string | Reliable |

All messages annotated `@domain(Handshake)`, `@type(0x01..0x03)`, `@revision(1)`.

---

### `schemas/shell.vexil`

**Namespace:** `malt.shell`

| Message | Fields | Priority |
|---------|--------|----------|
| `CommandStarted` | cmd @0 : string | Reliable |
| `CommandFinished` | exit_code @0 : i32, duration_us @1 : u64 | Reliable |
| `PromptReady` | cwd @0 : string | Reliable |
| `OutputChunk` | data @0 : bytes, command_tag @1 : optional\<string\> | Normal |

---

### `schemas/input.vexil`

**Namespace:** `malt.input`

**Key input union:**

```
@non_exhaustive
enum NamedKey {
    Enter @0  Escape @1  Tab @2  Backspace @3
    Insert @4  Delete @5  Home @6  End @7
    PageUp @8  PageDown @9
    Up @10  Down @11  Left @12  Right @13
}

@non_exhaustive
union KeyValue {
    Char     @0 { codepoint @0 : u32 }
    Named    @1 { key @0 : NamedKey }
    Function @2 { number @0 : u8 }
}
```

| Message | Fields | Priority |
|---------|--------|----------|
| `KeyEvent` | key @0 : KeyValue, modifiers @1 : KeyModifiers | Critical (not on bus — direct routing) |
| `MouseEvent` | x @0 : u16, y @1 : u16, kind @2 : MouseEventKind, button @3 : MouseButtonKind, modifiers @4 : KeyModifiers | Critical |
| `SignalInput` | signal @0 : SignalKind | Critical |
| `Resize` | cols @0 : u16, rows @1 : u16 | Critical |

---

### `schemas/mux.vexil`

**Namespace:** `malt.mux`

| Message | Fields | Priority |
|---------|--------|----------|
| `PaneCreated` | pane_id @0 : PaneId, kind @1 : PaneKind, title @2 : optional\<string\> | Reliable |
| `PaneDestroyed` | pane_id @0 : PaneId | Reliable |
| `LayoutChanged` | layout @0 : LayoutNode | Reliable |
| `SplitPane` | target @0 : PaneId, direction @1 : Direction, size @2 : SplitSize | Reliable |
| `ClosePane` | pane_id @0 : PaneId | Reliable |
| `FloatPane` | pane_id @0 : PaneId, x @1 : u16, y @2 : u16, width @3 : u16, height @4 : u16 | Reliable |
| `SwapPanes` | a @0 : PaneId, b @1 : PaneId | Reliable |
| `FocusDirection` | direction @0 : FocusDir | Reliable |

---

### `schemas/session.vexil`

**Namespace:** `malt.session`

| Message | Fields | Priority |
|---------|--------|----------|
| `CreateSession` | name @0 : optional\<string\>, isolation @1 : IsolationTier | Reliable |
| `AttachSession` | session_id @0 : SessionId | Reliable |
| `DetachSession` | session_id @0 : SessionId | Reliable |
| `ListSessions` | *(empty)* | Reliable |
| `SessionList` | sessions @0 : array\<SessionInfo\> | Reliable |

---

### `schemas/task.vexil`

**Namespace:** `malt.task`

```
@non_exhaustive
enum TaskState {
    Pending  @0
    Running  @1
    Complete @2
    Failed   @3
}
```

| Message | Fields | Priority |
|---------|--------|----------|
| `TaskCreate` | kind @0 : string, metadata @1 : optional\<string\> | Reliable |
| `TaskStatus` | task_id @0 : u32, state @1 : TaskState, progress @2 : optional\<u8\> | Reliable |
| `TaskComplete` | task_id @0 : u32, result @1 : result\<string, string\> | Reliable |

---

### `schemas/render.vexil`

**Namespace:** `malt.render`

**RenderCommand union (public, versioned, stable):**

```
@non_exhaustive
union RenderCommand {
    SetCursor    @0 { x @0 : u16  y @1 : u16 }
    SetClip      @1 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16 }
    ClearClip    @2 {}
    DrawText     @3 { x @0 : u16  y @1 : u16  text @2 : string  style @3 : ResolvedStyle }
    DrawRect     @4 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16  style @4 : ResolvedStyle }
    DrawBorder   @5 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16  style @4 : ResolvedStyle }
    DrawLine     @6 { x1 @0 : u16  y1 @1 : u16  x2 @2 : u16  y2 @3 : u16  style @4 : ResolvedStyle }
    DrawImage    @7 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16  data @4 : bytes  format @5 : ImageFormat }
    ScrollRegion @8 { x @0 : u16  y @1 : u16  width @2 : u16  height @3 : u16  delta @4 : i16 }
    PushLayer    @9 {}
    PopLayer     @10 {}
    WriteRaw     @11 { data @0 : bytes }
    Clear        @12 {}
    Flush        @13 {}
}

enum ImageFormat {
    Rgba @0
    Png  @1
    Sixel @2
}
```

| Message | Fields | Priority |
|---------|--------|----------|
| `RenderBatch` | frame_seq @0 : u64, commands @1 : array\<RenderCommand\> | High |
| `FrameAck` | frame_seq @0 : u64 | High |
| `InitialState` | frame_seq @0 : u64, layout @1 : LayoutNode, panes @2 : array\<ResolvedPane\>, commands @3 : array\<RenderCommand\> | Reliable |
| `SyncRequest` | *(empty)* | Reliable |
| `SlowClientDisconnect` | reason @0 : string | Reliable |
| `ScrollbackRequest` | pane_id @0 : PaneId, lines @1 : @varint u32 | Normal |
| `ScrollbackResponse` | pane_id @0 : PaneId, data @1 : bytes | Normal |

---

### `schemas/frame_element.vexil`

**Namespace:** `malt.frame_element`

Internal, unstable. Only core structural variants for Phase 1 — rich widgets (Diff, ProgressBar, Sparkline, etc.) added in Phase 3.

```
@non_exhaustive
union FrameElement {
    Text          @0 { text @0 : string  style @1 : ResolvedStyle }
    Paragraph     @1 { lines @0 : array<string>  style @1 : ResolvedStyle }
    Empty         @2 {}
    Split         @3 { direction @0 : Direction
                       sizes @1 : array<SplitSize>
                       children @2 : array<FrameElement> }
    Stack         @4 { children @0 : array<FrameElement> }
    VtPassthrough @5 { data @0 : bytes }
    Custom        @6 { type_id @0 : string  data @1 : bytes  fallback @2 : optional<FrameElement> }
}
```

Recursive: `Split.children`, `Stack.children`, `Custom.fallback` reference `FrameElement`.

---

### `schemas/system.vexil`

**Namespace:** `malt.system`

```
@non_exhaustive
enum PluginEventType {
    Loaded   @0
    Unloaded @1
    Error    @2
    Log      @3
}
```

| Message | Fields | Priority |
|---------|--------|----------|
| `StructuredOutput` | kind @0 : string, content @1 : bytes | Reliable |
| `PluginEvent` | plugin_id @0 : string, event_type @1 : PluginEventType, data @2 : optional\<bytes\> | Low |
| `Diagnostic` | severity @0 : Severity, message @1 : string, source @2 : optional\<string\> | Low |
| `Heartbeat` | *(empty)* | Low |
| `Error` | code @0 : @varint u32, message @1 : string, context @2 : optional\<string\> | Reliable |

---

### `schemas/elevate.vexil`

**Namespace:** `malt.elevate`

Restricted schema for the elevated helper binary. Minimal surface area.

```
message ElevateHello {
    nonce   @0 : u64
    version @1 : u32
}

message ElevateHelloAck {
    nonce    @0 : u64
    accepted @1 : bool
    reason   @2 : optional<string>
}

@non_exhaustive
union ElevateRequest {
    CreateNamespace @0 { pid @0 : u32  tier @1 : IsolationTier }
    MountOverlay    @1 { lower @0 : string  upper @1 : string  merged @2 : string }
    SetCgroup       @2 { pid @0 : u32  memory_mb @1 : u32  cpu_pct @2 : u16 }
    BindPort        @3 { port @0 : u16  socket_path @1 : string }
}

message ElevateResponse {
    request_id @0 : u32
    result     @1 : result<bytes, string>
}

message ElevateShutdown {
    reason @0 : string
}
```

---

### `schemas/persist/session.vexil`

**Namespace:** `malt.persist.session`

```
message PersistedSession {
    id             @0 : SessionId
    name           @1 : optional<string>
    layout         @2 : LayoutNode
    focus          @3 : PaneId
    panes          @4 : map<u32, PersistedPane>   // key is PaneId (see VEXIL_GAPS.md)
    theme          @5 : optional<ThemeOverride>
    group          @6 : optional<GroupId>
    isolation      @7 : IsolationTier
    schema_version @8 : u32
}

message PersistedPane {
    cwd       @0 : string
    title     @1 : optional<string>
    pane_type @2 : PersistedPaneType
}

@non_exhaustive
union PersistedPaneType {
    Shell  @0 { shell_path @0 : string }
    App    @1 { app_id @0 : string  config @1 : optional<bytes> }
    Compat @2 { program @0 : string  args @1 : array<string> }
}
```

---

### `schemas/persist/daemon.vexil`

**Namespace:** `malt.persist.daemon`

```
message DaemonState {
    sessions        @0 : array<SessionId>
    active_groups   @1 : array<GroupState>
    next_session_id @2 : u32
    next_pane_id    @3 : u32
}

message GroupState {
    id          @0 : GroupId
    name        @1 : string
    policy      @2 : optional<bytes>
    session_ids @3 : array<SessionId>
}
```

---

### `schemas/persist/layout.vexil`

**Namespace:** `malt.persist.layout`

```
message PersistedLayout {
    name   @0 : string
    layout @1 : LayoutNode
}
```

---

## Vexil-Lang Gaps

Documented in `VEXIL_GAPS.md` in the malt repo root.

### Gap 1: Map keys don't accept newtypes

**Problem:** `map<PaneId, PersistedPane>` is invalid. Must use `map<u32, PersistedPane>`.

**Impact:** Persistence schemas lose type safety on map keys. The generated Rust code uses raw `u32` keys instead of `PaneId`.

**Workaround:** Use raw primitive type for map keys, document with `@doc("key is PaneId")`.

**Requested fix:** Newtypes wrapping valid map key types (`u8`–`u64`, `i8`–`i64`, `string`, `bytes`, `uuid`, `enum`, `flags`) should themselves be valid map key types. The wire encoding is identical to the inner type.

### Gap 2: No custom/user-defined annotation support

**Problem:** Cannot attach arbitrary metadata to declarations. VNP needs to associate priority class, routing hints, and other protocol metadata with message types.

**Impact:** Priority class (`Critical`, `Reliable`, `High`, `Normal`, `Low`) must be maintained as a separate Rust const table rather than co-located with the schema definition.

**Workaround:** Document priority via `@doc("Priority: Critical")` and maintain a `priority_of()` function in `malt-protocol` Rust code.

**Requested fix:** Support user-defined annotations, e.g. `@meta(key, value)` or a pluggable annotation registry. Codegen backends should emit custom annotations as associated constants or a queryable metadata map.

### Gap 3: Potential `None` variant conflict in Rust codegen

**Problem:** Enum variants named `None` (e.g., `ColorDepth::None`) may conflict with `Option::None` in generated Rust code, depending on codegen output and usage context.

**Impact:** Several VNP enums use `None` as a variant name (`ColorDepth`, `UnicodeLevel`, `ImageProtocol`).

**Workaround:** Verify codegen output. If conflicts arise, rename variants to `Off` or `Unsupported`.

**Requested fix:** Rust codegen should handle `None` variant names safely — either by qualifying usage or by emitting a warning if the name collides with Rust builtins.

### Gap 4: No standard library types

**Problem:** Common semantic types like `Duration`, `IpAddr`, `SemVer`, `Url` are listed as future `vexil.std` additions but not available today.

**Impact:** VNP schemas use raw primitives (`u64` for microsecond durations, `string` for version strings, `i64` for timestamps). Type safety and documentation suffer.

**Workaround:** Use raw types with `@doc` annotations explaining semantics.

**Requested fix:** Implement `vexil.std` with at least: `Duration` (i64 microseconds), `Timestamp` (i64 microseconds since epoch), `SemVer` (string with validation).

---

## Key Design Decisions

1. **RenderCommand is a union, not separate messages.** `RenderBatch` carries `array<RenderCommand>` — one message per frame tick with packed drawing instructions rather than many messages on the bus.

2. **FrameElement in separate file.** It's internal/unstable and evolves independently from stable domain schemas. Nothing imports from it.

3. **KeyEvent uses a union for key values.** Three shapes (Unicode char, named key, function key) modeled cleanly as `KeyValue` union rather than a flat 100+ variant enum.

4. **Elevate uses `result<bytes, string>` for responses.** The success payload varies by request type, so `bytes` keeps the schema minimal. Error is always human-readable.

5. **FrameElement starts minimal.** Only core structural variants (Text, Paragraph, Empty, Split, Stack, VtPassthrough, Custom) for Phase 1. Rich widgets added in Phase 3. Union is `@non_exhaustive`.

6. **Envelope defined as Vexil schema with golden-byte verification.** Schema-driven per project principles. If Vexil flush rules produce unexpected bit layout, documented as a gap.

---

## Message Inventory Summary

| Domain | Messages | Types/Enums/Unions |
|--------|----------|--------------------|
| Common (shared) | 5 messages | 4 newtypes, 11 enums, 1 flags, 2 unions |
| Envelope | 1 message | — |
| Handshake | 3 messages | — |
| Shell | 4 messages | — |
| Input | 4 messages | 1 enum, 1 union |
| Mux | 8 messages | — |
| Session | 5 messages | — |
| Task | 3 messages | 1 enum |
| Render | 7 messages | 1 union (14 variants), 1 enum |
| FrameElement | — | 1 union (7 variants) |
| System | 5 messages | 1 enum |
| Elevate | 5 messages | 1 union (4 variants) |
| Persist | 4 messages | 1 union (3 variants) |
| **Total** | **54 messages** | **4 newtypes, 15 enums, 1 flags, 6 unions** |

---

## Testing Strategy

1. **Schema compilation:** Every `.vexil` file must compile cleanly with `vexilc`. CI gate.

2. **Envelope golden-byte test:** Assert that the Vexil-generated Envelope encoding matches the exact bit layout from architecture.md §6. This is the most critical test — it validates that Vexil sub-byte packing produces the expected wire format.

3. **Roundtrip tests per domain:** For each message type, encode a representative value, decode it, assert equality. Generated by the test harness or hand-written for complex types (LayoutNode recursive tree, RenderCommand union).

4. **Cross-file import tests:** Verify that domain schemas correctly import and reference types from `common.vexil`.

5. **Persistence roundtrip:** Encode `PersistedSession` with all fields populated (including recursive `LayoutNode`), write to `.vx` text format, read back, assert equality.
