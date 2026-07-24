# VNP Protocol Specification

> **Status:** Draft v0.1. Authoritative reference for the VNP (Vexil Native
> Protocol) wire format. Derived from `specs/architecture.md` §6, Phase 1
> schema design, implemented framing code, and envelope schema.

---

## 1. Overview

VNP (Vexil Native Protocol) is the typed message protocol that replaces
terminal escape codes in MALT. Every inter-component message — between daemon
and clients, between daemon subsystems, and between daemon and elevated
helper — is a VNP message.

**Key properties:**

- **Typed.** Every message is defined by a Vexil schema (`.vexil` file).
  Wire encoding is Vexil bitpack — deterministic, LSB-first, compact.
- **Schema-driven.** Message types, field layouts, and evolution rules are
  derived from schemas. Codegen produces Rust types, encode/decode
  implementations, and documentation from a single source of truth.
- **Deterministic.** Same message with same field values produces
  bit-identical bytes on every platform. This enables content-addressed
  message identity via BLAKE3 hash, replay detection, and cross-platform
  verification.
- **Evolvable.** New fields append without breaking existing decoders.
  Schema evolution follows formal rules (see §6). Wire version negotiation
  ensures peers operate at a mutually understood protocol level.
- **Transport-agnostic.** VNP defines framing and messages, not transport.
  Any reliable ordered byte stream works (see §7).

VNP exists because escape codes are ambiguous, stateful, and
parser-dependent. VNP messages are unambiguous, self-describing (via schema
hash), and decodable by any implementation with the matching schema version.

---

## 2. Wire Format

VNP messages are structured in three layers. From outermost to innermost:

```
┌─────────────────────────────────────────────────┐
│  Layer 1: Framing                               │
│  [4-byte LE length] [1-byte flags] [payload]    │
│  ┌─────────────────────────────────────────────┐ │
│  │  Layer 2: Envelope                          │ │
│  │  Bit-packed header (12-16 bytes)            │ │
│  │  ┌─────────────────────────────────────────┐│ │
│  │  │  Layer 3: Payload                       ││ │
│  │  │  Vexil bitpack-encoded message body     ││ │
│  │  └─────────────────────────────────────────┘│ │
│  └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

### 2.1 Framing

Every VNP frame on the wire has this layout:

```
Byte offset:  0       1       2       3       4         5 .. 4+N
            ┌───────┬───────┬───────┬───────┬─────────┬──────────────┐
            │ len[0]│ len[1]│ len[2]│ len[3]│  flags  │   payload    │
            └───────┴───────┴───────┴───────┴─────────┴──────────────┘
             ◄──── 4-byte LE u32 ────►  1 byte    N bytes
```

- **Length** (4 bytes, little-endian u32): The byte count of the `payload`
  field only. Does not include the length or flags bytes themselves. A
  length of 0 is valid (empty payload).

- **Flags** (1 byte): Bit field controlling frame interpretation.

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | `COMPRESSED` | Payload is zstd-compressed. Decompress before decoding envelope + payload. |
| 1 | `JSON_ENCODED` | Payload uses JSON encoding instead of Vexil bitpack. For debugging only — not used in production traffic. |
| 2 | `CONTINUATION` | This frame is a continuation of a multi-frame logical message. The receiver must buffer and reassemble all continuation frames before dispatching. |
| 3-7 | Reserved | Must be zero. A receiver that sees any reserved bit set must reject the frame with a `ReservedFlagsSet` error. |

- **Payload** (N bytes): The envelope header followed by the message body
  (or, when `COMPRESSED` is set, the compressed form of both).

**Size limits:**

| Limit | Value | Purpose |
|-------|-------|---------|
| Protocol maximum | 16 MiB (16,777,216 bytes) | Absolute ceiling. No frame may exceed this regardless of configuration. |
| Default maximum | 64 KiB (65,536 bytes) | Applied when no transport-specific override is configured. |
| Transport overrides | Configurable per transport (e.g., 64 KiB for named pipes, 1 MiB for TCP) | Tuned to transport characteristics. |

A frame whose length field exceeds the receiver's configured maximum is
rejected with a `FrameTooLarge` error. The connection is not terminated —
the receiver discards the oversized frame and continues reading.

**Chunking:** When a logical message exceeds the frame size limit, the
sender splits it into multiple frames. All frames except the last carry the
`CONTINUATION` flag (bit 2). The receiver buffers continuation frames and
reassembles the complete payload before decoding the envelope and dispatching
the message. The reassembled payload must not exceed the protocol maximum
(16 MiB). OutputChunk messages from shell commands are capped at the
per-transport frame size at emission time — MASH emits chunks at PTY read
boundaries, so individual OutputChunks do not approach 16 MiB in practice.

### 2.2 Envelope

The envelope is a Vexil bitpack-encoded header that precedes every message
payload. It is defined in `schemas/envelope.vexil` as a Vexil message type
and follows standard Vexil bitpack encoding rules (fields in declaration
order, LSB-first, packed at bit boundaries).

**Schema definition:**

```
message Envelope {
    wire_version @0 : u4          // 4 bits
    domain       @1 : u4          // 4 bits
    msg_type     @2 : u7          // 7 bits
    session_id   @3 : u32         // 32 bits
    timestamp    @4 : u48         // 48 bits
    msg_id       @5 : optional<u32>  // 1-bit presence flag + 32 bits if present
}
```

**Bit layout (Vexil bitpack, LSB-first):**

```
Bit:   0    3 4    7 8      14 15                46 47                      94
      ┌──────┬──────┬─────────┬───────────────────┬─────────────────────────────┐
      │ wire │domain│msg_type │    session_id      │        timestamp            │
      │ ver  │      │         │     (32 bits)      │        (48 bits)            │
      │(4b)  │(4b)  │ (7b)    │                    │                             │
      └──────┴──────┴─────────┴───────────────────┴─────────────────────────────┘
Bit:  95   95  96                       127
      ┌──────┬───────────────────────────────┐
      │has_id│     msg_id (32b, if present)  │
      │(1b)  │                               │
      └──────┴───────────────────────────────┘
```

- **Total size without msg_id:** 96 bits = 12 bytes.
- **Total size with msg_id:** 128 bits = 16 bytes.

**Field semantics:**

| Field | Bits | Range | Description |
|-------|------|-------|-------------|
| `wire_version` | 4 | 0-15 | Wire format version. Distinct from the software protocol version exchanged in Hello/HelloAck. Used by the decoder to select the correct envelope layout. Current value: 0. |
| `domain` | 4 | 0-15 | Message domain. IDs 0-7 are assigned (see §3). IDs 8-15 are reserved for future domains. |
| `msg_type` | 7 | 0-127 | Message type within the domain. Each domain has its own 128-type namespace, giving 2,048 total message types across all domains. |
| `session_id` | 32 | 0-2^32-1 | Target session for bus routing. Value 0 means daemon-global (not scoped to any session). Monotonically increasing, never recycled within a daemon lifetime. |
| `timestamp` | 48 | 0-2^48-1 | Microseconds since daemon start. Wraps at approximately 8.9 years. Wall-clock conversion uses the `start_time_offset` field from HelloAck. Comparison across daemon restarts is invalid. |
| `msg_id` | 32 (optional) | 0-2^32-1 | Correlation ID for request/response pairing. Present when the 1-bit presence flag is set. Used by Session, Task, and Render domains for matching responses to requests. |

### 2.3 Payload

The payload immediately follows the envelope within the frame. It is a Vexil
bitpack-encoded message body whose schema is identified by the combination of
`domain` and `msg_type` from the envelope.

Encoding rules follow the Vexil bitpack specification:

- Fields are encoded in ordinal order (`@0`, `@1`, ...), LSB-first.
- Optional fields are preceded by a 1-bit presence flag.
- Arrays are length-prefixed (varint count, then elements).
- Strings are UTF-8, length-prefixed (varint byte count).
- Bytes are raw, length-prefixed (varint byte count).
- Unions encode a varint discriminant followed by the variant's fields.
- Newtypes encode identically to their inner type.

When `JSON_ENCODED` (flag bit 1) is set, the payload is a JSON object
instead of bitpack. This mode exists solely for debugging and protocol
inspection tools — production traffic always uses bitpack.

When `COMPRESSED` (flag bit 0) is set, the entire content after the flags
byte (envelope + payload together) is zstd-compressed. The receiver must
decompress before parsing the envelope.

---

## 3. Message Domains

VNP organizes messages into domains. Each domain occupies a 4-bit ID in the
envelope and has its own 7-bit message type namespace (128 types per domain).

### 3.1 Domain ID Assignments

| ID | Domain | Scope |
|----|--------|-------|
| 0 | Handshake | Connection setup, version negotiation, capability exchange |
| 1 | Shell | Command lifecycle, output streaming, prompt state |
| 2 | Input | Keyboard, mouse, signals, resize |
| 3 | Mux | Pane creation/destruction, layout changes, focus management |
| 4 | Session | Session creation, attachment, detachment, listing |
| 5 | Task | Task lifecycle (create, status, complete) |
| 6 | Render | Render commands, frame acknowledgment, sync, scrollback |
| 7 | System | Structured output, plugin events, diagnostics, heartbeat, errors |
| 8-15 | Reserved | Reserved for future domains. Must not be used by current implementations. |

Only the 8 wire domains (0-7) consume domain IDs. The Envelope, FrameElement,
Elevate, and Persist namespaces are schema-only organizational namespaces —
they do not consume domain IDs. Elevate uses its own restricted framing.
Persist schemas are stored via Vexil Store, not sent as VNP messages.

### 3.2 Message Type IDs per Domain

Type IDs are assigned via `@type(0xNN)` annotations in schema files. Type
IDs within each domain start at `0x01` and are assigned sequentially.

**Domain 0 — Handshake** (`schemas/handshake.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `Hello` | Client -> Daemon | Protocol version, client type, capabilities |
| 0x02 | `HelloAck` | Daemon -> Client | Negotiated version, session list, daemon start time offset |
| 0x03 | `VersionSkew` | Daemon -> Client | Version mismatch disconnect with expected range |

**Domain 1 — Shell** (`schemas/shell.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `CommandStarted` | Daemon -> Client | Command execution begun (command_id, cmd string) |
| 0x02 | `CommandFinished` | Daemon -> Client | Command completed (exit code, duration) |
| 0x03 | `PromptReady` | Daemon -> Client | Shell ready for input (cwd) |
| 0x04 | `OutputChunk` | Daemon -> Client | Streaming command output (data bytes, optional command tag) |

**Domain 2 — Input** (`schemas/input.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `KeyEvent` | Client -> Daemon | Keystroke (key value + modifiers) |
| 0x02 | `MouseEvent` | Client -> Daemon | Mouse action (position, kind, button, modifiers) |
| 0x03 | `SignalInput` | Client -> Daemon | Signal request (Int, Tstp, Quit, etc.) |
| 0x04 | `Resize` | Client -> Daemon | Terminal dimensions changed (cols, rows) |

**Domain 3 — Mux** (`schemas/mux.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `PaneCreated` | Daemon -> Client | New pane created (id, kind, title) |
| 0x02 | `PaneDestroyed` | Daemon -> Client | Pane removed |
| 0x03 | `LayoutChanged` | Daemon -> Client | Layout tree updated |
| 0x04 | `SplitPane` | Client -> Daemon | Request to split a pane |
| 0x05 | `ClosePane` | Client -> Daemon | Request to close a pane |
| 0x06 | `FloatPane` | Client -> Daemon | Request to float a pane |
| 0x07 | `SwapPanes` | Client -> Daemon | Request to swap two panes |
| 0x08 | `FocusDirection` | Client -> Daemon | Move focus in a direction |
| 0x09 | `ResizeSplit` | Client -> Daemon | Resize a split boundary |
| 0x0A | `SaveLayout` | Client -> Daemon | Save current layout by name |
| 0x0B | `LoadLayout` | Client -> Daemon | Restore a saved layout by name |

**Domain 4 — Session** (`schemas/session.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `CreateSession` | Client -> Daemon | Create a new session (name, isolation tier, group) |
| 0x02 | `AttachSession` | Client -> Daemon | Attach to an existing session |
| 0x03 | `DetachSession` | Client -> Daemon | Detach from a session |
| 0x04 | `ListSessions` | Client -> Daemon | Request session list |
| 0x05 | `SessionList` | Daemon -> Client | Session list response |
| 0x06 | `InputClaim` | Client -> Daemon | Claim input authority on a session |
| 0x07 | `InputAuthorityChanged` | Daemon -> Client | Input authority transferred |

**Domain 5 — Task** (`schemas/task.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `TaskCreate` | Client -> Daemon | Create a new task (kind, metadata) |
| 0x02 | `TaskStatus` | Daemon -> Client | Task state update (state, progress) |
| 0x03 | `TaskComplete` | Daemon -> Client | Task finished (result or error) |

**Domain 6 — Render** (`schemas/render.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `RenderBatch` | Daemon -> Client | Frame of render commands (frame_seq, command array) |
| 0x02 | `FrameAck` | Client -> Daemon | Client acknowledges processed frame |
| 0x03 | `InitialState` | Daemon -> Client | Full state snapshot for attach/re-sync |
| 0x04 | `SyncRequest` | Client -> Daemon | Request full re-sync |
| 0x05 | `SlowClientDisconnect` | Daemon -> Client | Disconnect due to client falling behind |
| 0x06 | `ScrollbackRequest` | Client -> Daemon | Request scrollback history for a pane |
| 0x07 | `ScrollbackResponse` | Daemon -> Client | Scrollback data response |

**Domain 7 — System** (`schemas/system.vexil`)

| Type ID | Message | Direction | Description |
|---------|---------|-----------|-------------|
| 0x01 | `StructuredOutput` | Daemon -> Client | Parsed command output (kind, content) |
| 0x02 | `PluginEvent` | Daemon -> Client | Plugin lifecycle event |
| 0x03 | `Diagnostic` | Daemon -> Client | Diagnostic message (severity, source) |
| 0x04 | `Heartbeat` | Bidirectional | Liveness probe (sequence number) |
| 0x05 | `Error` | Daemon -> Client | Error response (code, message, context) |

---

## 4. Priority Classes

Every VNP message has an assigned priority class that determines its bus
delivery behavior. Priority is a property of the message type, not the
individual message instance.

### 4.1 Priority Definitions

| Priority | Drop Policy | Bus Behavior |
|----------|-------------|-------------|
| **Critical** | Never dropped | Delivered via separate per-subscriber inbox, bypassing the ring buffer. Dispatched asynchronously (non-blocking). Subscribers poll their Critical inbox before draining the ring buffer. Rare, small messages only. |
| **Reliable** | Never evicted from ring buffer | Inserted into the ring buffer with a never-evict marker. When the buffer is full, the oldest evictable (Normal/Low) message is removed. Reliable messages never block the producer. Subject to saturation guards (see below). |
| **High** | Oldest overwritten | Evictable, but only by newer High messages. Renderer Host can reconstruct from the latest FrameElement tree, so loss of older frames is tolerable. |
| **Normal** | Oldest dropped per subscriber | First to be evicted when the buffer is full. Dropped message count tracked per subscriber for diagnostics. |
| **Low** | Oldest dropped per subscriber | Same eviction behavior as Normal. Background work that tolerates delay and loss. |

### 4.2 Message-to-Priority Mapping

| Domain | Message | Priority |
|--------|---------|----------|
| Handshake | Hello | Reliable |
| Handshake | HelloAck | Reliable |
| Handshake | VersionSkew | Reliable |
| Shell | CommandStarted | Reliable |
| Shell | CommandFinished | Reliable |
| Shell | PromptReady | Reliable |
| Shell | OutputChunk | Normal |
| Input | KeyEvent | Critical (direct routing, not on bus) |
| Input | MouseEvent | Critical |
| Input | SignalInput | Critical |
| Input | Resize | Critical |
| Mux | PaneCreated | Reliable |
| Mux | PaneDestroyed | Reliable |
| Mux | LayoutChanged | Reliable |
| Mux | SplitPane | Reliable |
| Mux | ClosePane | Reliable |
| Mux | FloatPane | Reliable |
| Mux | SwapPanes | Reliable |
| Mux | FocusDirection | Reliable |
| Mux | ResizeSplit | Reliable |
| Mux | SaveLayout | Reliable |
| Mux | LoadLayout | Reliable |
| Session | CreateSession | Reliable |
| Session | AttachSession | Reliable |
| Session | DetachSession | Reliable |
| Session | ListSessions | Reliable |
| Session | SessionList | Reliable |
| Session | InputClaim | Reliable |
| Session | InputAuthorityChanged | Reliable |
| Task | TaskCreate | Reliable |
| Task | TaskStatus | Reliable |
| Task | TaskComplete | Reliable |
| Render | RenderBatch | High |
| Render | FrameAck | Normal |
| Render | InitialState | Reliable |
| Render | SyncRequest | Reliable |
| Render | SlowClientDisconnect | Reliable |
| Render | ScrollbackRequest | Normal |
| Render | ScrollbackResponse | Normal |
| System | StructuredOutput | Reliable |
| System | PluginEvent | Low |
| System | Diagnostic | Low |
| System | Heartbeat | Low |
| System | Error | Reliable |

Note: InputEvent (KeyEvent) is not on the message bus — it flows directly
from daemon core to the focused pane's owner (MASH, App, or Compat
Translator) on the latency-critical hot path.

### 4.3 Reliable Saturation Guards

To prevent Reliable messages from filling the entire ring buffer and starving
Normal/Low delivery, two saturation guards apply:

1. **Per-producer cap (25%):** Each producer may hold at most 25% of a
   subscriber's buffer capacity as Reliable entries. If exceeded, the oldest
   Reliable message from that producer is demoted to Normal (becoming
   evictable).

2. **Global cap (60%):** Total Reliable entries across all producers may not
   exceed 60% of buffer capacity. When the global cap is reached, the oldest
   Reliable entry regardless of producer is demoted. Without this cap, four
   producers each at 25% could collectively fill 100% of the buffer with
   never-evict entries.

The bus tracks `reliable_pressure` per subscriber (ratio of Reliable entries
to total capacity). A diagnostic is emitted at 50% pressure.

---

## 5. Connection Lifecycle

### 5.1 Handshake Sequence

```
Client                                     Daemon
  │                                          │
  │──── [transport connect] ────────────────►│
  │                                          │
  │──── Hello ─────────────────────────────►│
  │     { version, client_type,              │
  │       capabilities }                     │
  │                                          │
  │◄──── HelloAck ─────────────────────────│
  │      { negotiated_version,               │
  │        sessions[], start_time_offset }   │
  │                                          │
  │──── AttachSession / CreateSession ─────►│
  │     { session_id / name, isolation,      │
  │       authority }                        │
  │                                          │
  │◄──── InitialState ─────────────────────│
  │      { frame_seq, layout, panes[],       │
  │        commands[] }                      │
  │                                          │
  │◄───► Bidirectional message flow ───────►│
  │                                          │
```

1. **Transport connect.** Client opens a transport connection (named pipe,
   Unix socket, TCP+TLS, WebSocket+TLS, or in-process channel).

2. **Hello.** Client sends a `Hello` message declaring:
   - `version` (u32): Maximum wire protocol version the client supports.
   - `client_type` (string): Client identifier (e.g., `"maltty"`, `"malt-tui"`,
     `"malt-web"`).
   - `capabilities` (ClientCapabilities): Structured capability declaration
     including color depth, unicode level, image protocol support, overlay
     support, VT passthrough support, and preferred max FPS.

3. **HelloAck.** Daemon responds with:
   - `negotiated_version` (u32): The minimum of client and daemon supported
     versions. Both sides operate at this version for the connection lifetime.
   - `sessions` (array of SessionInfo): List of existing sessions the client
     may attach to.
   - `start_time_offset` (i64): Daemon's start time as microseconds since
     Unix epoch. Clients use this to convert envelope timestamps to
     wall-clock time for log correlation.

4. **Session binding.** Client sends either:
   - `AttachSession` to connect to an existing session, or
   - `CreateSession` to start a new one.

5. **Initial state.** For AttachSession, the daemon responds with an
   `InitialState` snapshot containing the current full FrameElement tree,
   resolved layout, and a `frame_seq` baseline. RenderCommand deltas begin
   from `frame_seq + 1`. The snapshot is taken under the executor's
   single-threaded guarantee — no FrameElement updates can occur between
   snapshot and first delta.

6. **Bidirectional flow.** Messages flow in both directions according to
   their domain semantics.

### 5.2 Version Negotiation

Both sides declare the maximum wire version they support. They operate at
the minimum of the two:

```
negotiated_version = min(client.version, daemon.version)
```

Wire versions are additive — version N+1 is a strict superset of version N.
A decoder at version N reads the first N fields of any message and ignores
trailing bytes from a version N+1 encoder.

**Compatibility window:** The daemon at version N must support clients at
version N and N-1. Clients at version N-2 or older are disconnected
immediately after Hello/HelloAck with a `VersionSkew` error containing the
expected version range.

### 5.3 Capability Exchange

The Renderer Host uses negotiated capabilities to produce per-client
RenderCommand streams. Capabilities are per-connection, not per-session —
two clients attached to the same session can receive different rendering
fidelity:

- Clients that don't support images never receive `DrawImage` commands;
  the Renderer Host substitutes a text fallback.
- Clients that don't support overlays never receive `PushLayer`/`PopLayer`.
- The `max_fps` field caps the daemon's RenderBatch emission rate for that
  client.

### 5.4 Client Backpressure

**Frame sequencing:** Every `RenderBatch` carries a monotonic `frame_seq`
counter. Clients acknowledge processed frames via `FrameAck { frame_seq }`.

**Slow client detection:** If a client's unacknowledged frame count exceeds
30 frames (~500ms at 60 FPS), the Renderer Host stops producing
RenderCommands for that client (marked "lagging"). Production resumes when
the client's FrameAck reaches within 5 of the current frame.

**Slow client disconnect:** If a client has not sent a FrameAck for 10
seconds, the daemon disconnects it with a `SlowClientDisconnect` message.
The client can reconnect and re-sync.

**Re-sync:** A client can request a full re-sync at any time via
`SyncRequest`. The daemon responds with a fresh `InitialState` snapshot and
resets the delta stream. This is the recovery path after a slow-client
disconnect.

### 5.5 Multi-Client Sessions

Multiple clients can attach to the same session simultaneously. One client
at a time holds **input authority** — its keystrokes reach the focused pane.
Other attached clients are observers: they receive RenderCommand streams but
their input is silently discarded.

- **Default:** Most recently attached client gets input authority.
- **Observe mode:** `AttachSession` with `authority: Observe` attaches
  without claiming authority.
- **Transfer:** `InputClaim` message transfers authority to the requesting
  client.
- **Release:** When the authoritative client detaches, authority transfers to
  the next attached client by attachment order.

---

## 6. Schema Evolution

VNP schemas evolve over time as the protocol gains capabilities. Evolution
follows formal rules that preserve compatibility where possible and require
explicit version bumps where not.

### 6.1 Evolution Rules

| Operation | Compatibility | Mechanism |
|-----------|--------------|-----------|
| Append new field to message | Forward + backward compatible | Decoder ignores trailing bytes; encoder uses defaults for missing fields |
| Add variant to union/enum type | Forward compatible only | Old decoders see unknown variant, use declared fallback or reject |
| Change field from required to optional | **Breaking** | Wire version bump required |
| Remove or reorder existing fields | **Breaking** | Wire version bump required |
| Expand a bit-width field (e.g., 3-bit -> 4-bit) | **Breaking** | Wire version bump; envelope layout changes |
| Add new domain to domain field | **Breaking** (field too narrow) | Use reserved domain values or bump wire version |

### 6.2 Bitpack Forward Compatibility

New fields are always appended at the end of the message schema. Bitpack
reads fields in declaration order; a decoder at wire version N reads the
first N fields and ignores trailing bytes it does not recognize. This
preserves the "same fields produce bit-identical prefix" property: the
shared field prefix between version N and version N+1 encodes identically.
New fields have default values that preserve version N semantics when absent.

### 6.3 Schema Identity

Each message type's schema is identified by its BLAKE3 hash. This hash
covers the schema definition (field names, types, ordinals, annotations)
and changes whenever the schema changes.

**Runtime mismatch handling:** If a received message's BLAKE3 schema hash
does not match the expected hash for that message type at the negotiated
wire version:

1. The message is rejected with a diagnostic error (not silently dropped).
2. On sustained mismatch (>10 rejected messages in 5 seconds), the daemon
   proactively disconnects the client with a typed `VersionSkew` error
   including the expected version range.
3. The `VersionSkew` error enables client-side "please update" UX.

### 6.4 Wire Version Bump Protocol

When a breaking change is necessary:

1. The new wire version is assigned (increment of the 4-bit `wire_version`
   field).
2. Schemas for the new version are published alongside the old.
3. The daemon supports both the current and previous wire version
   simultaneously (N and N-1 compatibility window).
4. Clients negotiate the highest mutually supported version via
   Hello/HelloAck.
5. After the compatibility window, support for version N-2 is dropped.

---

## 7. Transport Requirements

VNP is transport-agnostic. It requires only a reliable, ordered byte stream.
The protocol defines framing and message semantics; the transport provides
delivery.

### 7.1 Supported Transports

| Transport | Platform | Notes |
|-----------|----------|-------|
| **Named pipes** | Windows | Primary local transport. Tokio does not support Unix sockets on Windows; named pipes are the correct choice. Default frame size: 64 KiB. |
| **Unix sockets** | Linux, macOS | Primary local transport on Unix systems. |
| **TCP + TLS** | All (remote) | For remote access (e.g., over Tailscale). **TLS is mandatory** — VNP payloads contain keystrokes and command output. Self-signed certificates with TOFU pinning for personal use; CA-signed for shared infrastructure. |
| **WebSocket + TLS** | Browser | For the `malt-web` SvelteKit client. Same TLS requirement as TCP. WebSocket framing wraps VNP frames. |
| **In-process channel** | All (embedded) | For daemon-internal subsystem communication. No serialization overhead — messages are passed by reference. Same VNP message types, but zero-copy. |

### 7.2 Transport-Agnostic Design

Transport selection does not affect message semantics. The same VNP messages
flow over named pipes, Unix sockets, TCP, WebSocket, and in-process
channels. The only transport-visible differences are:

- **Frame size limits** may vary per transport (configured at connection
  time).
- **TLS wrapping** is mandatory for network transports (TCP, WebSocket) and
  not applicable to local transports (named pipe, Unix socket, in-process).
- **Authentication mechanism** differs: local transports use OS peer
  credentials; remote transports use token-based authentication.

---

## 8. Security

### 8.1 Transport Security

- **TLS is mandatory for all network transports.** TCP and WebSocket
  connections without TLS must be rejected. VNP payloads contain keystrokes,
  command output, and session state — plaintext transmission over a network
  is not acceptable.
- **Local transports** (named pipe, Unix socket) rely on OS-level access
  control. Authentication uses OS peer credentials (same user, zero
  configuration).
- **Remote transports** require token-based authentication. The API Gateway
  is the only external-facing surface; internal subsystems are not
  network-accessible.

### 8.2 Elevate Channel Security

The `malt-elevate` helper binary communicates with the daemon over a
dedicated channel using a restricted VNP schema (`elevate.vexil`). Security
properties:

- **Nonce-based authentication.** `ElevateHello` carries a cryptographic
  nonce. The helper validates the nonce before accepting any requests. This
  prevents unauthorized processes from connecting to the elevate socket.
- **Restricted schema.** The helper accepts only ~6 message types defined in
  `elevate.vexil`. It does not import the full VNP domain system. The audit
  surface is a single readable file.
- **Independent validation.** The helper validates every request
  independently — it does not trust the daemon. Even if the daemon is
  compromised, the helper's fixed operation set limits the damage surface.

### 8.3 Wire Format Integrity

Vexil bitpack encoding is deterministic: same message, same field values
produce bit-identical bytes. This enables:

- Content-addressed message identity via BLAKE3 hash.
- Replay detection.
- Cross-platform verification — a message encoded on Linux decodes
  identically on Windows and macOS.

### 8.4 Schema Hash Enforcement

Schema hash mismatch is treated as a security-relevant event. A client
sending messages that do not match the expected schema at the negotiated
wire version is either buggy or malicious. The escalation path (reject
individual messages, then disconnect after sustained mismatch) prevents
both accidental and intentional protocol abuse.

---

## References

- `specs/architecture.md` — Full architecture specification. Section 6 (VNP),
  Section 2 (bus priority, flow control), Section 14 (security model).
- `specs/phase1-vnp-schema-design.md` — Schema definitions, domain ID table,
  message inventory, priority assignments.
- `crates/malt-protocol/src/framing.rs` — Reference implementation of the
  framing layer.
- `schemas/envelope.vexil` — Envelope schema definition.
