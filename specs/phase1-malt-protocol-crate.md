# Phase 1 Sub-Project 2: `malt-protocol` Crate

## Goal

Build the `malt-protocol` Rust crate — the L0 foundation that provides VNP message types (Vexil-generated), wire framing, envelope encode/decode, and priority mapping. Every other MALT crate depends on this.

## Architecture

`malt-protocol` is the single entry point for VNP types. It calls `vexilc build` at compile time to generate Rust types from the `.vexil` schemas, then re-exports them in domain-organized modules. The framing layer (hand-written) handles wire transport below the Vexil serialization layer. The crate has zero internal workspace dependencies — only `vexil-runtime` (external) and `thiserror` (error types).

## Spec Reference

- `malt/specs/architecture.md` §6 (VNP), §2 (Bus priority classes)
- `malt/specs/phase1-vnp-schema-design.md` (schema definitions, domain ID table, priority assignments)

---

## Crate Structure

```
orix/malt/crates/malt-protocol/
  Cargo.toml
  build.rs                        # Invokes vexilc build on schemas/
  src/
    lib.rs                        # Re-exports: framing, envelope, priority, domain modules
    framing.rs                    # Frame, FrameFlags, FrameReader, FrameWriter
    priority.rs                   # Priority enum, priority_of() mapping
    envelope.rs                   # Re-export + helpers for generated Envelope type
    common.rs                     # Re-export of generated malt.common types
    handshake.rs                  # Re-export of generated malt.handshake types
    shell.rs                      # Re-export of generated malt.shell types
    input.rs                      # Re-export of generated malt.input types
    mux.rs                        # Re-export of generated malt.mux types
    session.rs                    # Re-export of generated malt.session types
    task.rs                       # Re-export of generated malt.task types
    render.rs                     # Re-export of generated malt.render types
    frame_element.rs              # Re-export of generated malt.frame_element types
    system.rs                     # Re-export of generated malt.system types
    elevate.rs                    # Re-export of generated malt.elevate types
    persist/
      mod.rs
      session.rs                  # Re-export of generated malt.persist.session types
      daemon.rs                   # Re-export of generated malt.persist.daemon types
      layout.rs                   # Re-export of generated malt.persist.layout types
  tests/
    envelope_golden.rs            # Golden-byte test for envelope bit layout
    framing.rs                    # Framing roundtrip tests
    priority.rs                   # Exhaustive priority mapping test
    roundtrip.rs                  # Message encode/decode roundtrip per domain
```

Domain module files are thin wrappers: `include!` the vexilc-generated code from `$OUT_DIR` and re-export. Most are under 10 lines. Hand-written helpers only where needed (e.g., `envelope.rs` has `encode_message`/`decode_envelope` convenience functions).

---

## Dependencies

```toml
[dependencies]
vexil-runtime = { path = "../../vexil-lang/crates/vexil-runtime" }
thiserror = "2"

[build-dependencies]
# vexilc invoked as CLI tool via build.rs — not a library dependency
```

**No other dependencies.** No async runtime (framing uses `std::io`), no serde (JSON handled by API gateway), no zstd (compression handled by transport layer in daemon).

---

## Build Integration

`build.rs` invokes `vexilc build` at compile time:

1. Locates `vexilc` on `PATH` or at `VEXILC_PATH` env var
2. Runs `vexilc build <root>.vexil --include ../../schemas/ --output $OUT_DIR --target rust` for each schema
3. Generates one `.rs` file per schema namespace in `$OUT_DIR`
4. Domain modules `include!` the generated files

The `vexilc` output filename convention needs to be verified against actual `vexilc build` output (likely `malt.shell.rs`, `malt.common.rs`, etc.).

---

## Framing Layer

`framing.rs` — hand-written Rust. Implements the wire frame format defined in architecture.md §6.

### Frame Wire Layout

```
[4 bytes: payload length (LE u32)] [1 byte: flags] [payload bytes]
```

The payload contains the envelope + message body. Framing is agnostic to payload contents.

### FrameFlags (1 byte, bitfield)

| Bit | Name | Meaning |
|-----|------|---------|
| 0 | compressed | 0 = none, 1 = zstd (flag only — caller decompresses) |
| 1 | json_encoded | 0 = bitpack, 1 = JSON (flag only — caller handles) |
| 2 | continuation | 0 = final frame, 1 = more chunks follow |
| 3-7 | reserved | Must be 0 |

### Public API

```rust
pub struct Frame {
    pub flags: FrameFlags,
    pub payload: Vec<u8>,
}

pub struct FrameFlags(u8);

impl FrameFlags {
    pub fn new() -> Self;
    pub fn compressed(&self) -> bool;
    pub fn set_compressed(&mut self, v: bool);
    pub fn json_encoded(&self) -> bool;
    pub fn set_json_encoded(&mut self, v: bool);
    pub fn continuation(&self) -> bool;
    pub fn set_continuation(&mut self, v: bool);
}

/// Reads frames from a std::io::Read source.
pub struct FrameReader<R: Read> {
    inner: R,
    max_frame_size: u32,
}

impl<R: Read> FrameReader<R> {
    pub fn new(reader: R) -> Self;
    pub fn with_max_frame_size(reader: R, max: u32) -> Self;
    pub fn read_frame(&mut self) -> Result<Frame, FrameError>;
}

/// Writes frames to a std::io::Write sink.
pub struct FrameWriter<W: Write> {
    inner: W,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(writer: W) -> Self;
    pub fn write_frame(&mut self, frame: &Frame) -> Result<(), FrameError>;
}
```

### Frame Size Limits

- Default max frame size: 64 KiB (configurable per transport)
- Protocol maximum: 16 MiB (hard limit, not configurable)
- `FrameReader` rejects frames exceeding its configured max

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame size {size} exceeds maximum {max}")]
    FrameTooLarge { size: u32, max: u32 },

    #[error("unexpected end of stream")]
    UnexpectedEof,

    #[error("reserved flags bits are set: {0:#04x}")]
    ReservedFlagsSet(u8),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

### No Async Runtime Dependency

`FrameReader`/`FrameWriter` use `std::io::Read`/`Write`. The daemon wraps them with tokio's `AsyncReadExt`/`AsyncWriteExt` via `tokio::io::BufReader`. This keeps `malt-protocol` runtime-agnostic.

---

## Envelope Integration

`envelope.rs` re-exports the Vexil-generated `Envelope` type from `envelope.vexil` and adds hand-written convenience functions.

### Public API

```rust
// Re-exported from generated code:
pub use generated::Envelope;

/// Encode an envelope + message payload into bytes suitable for framing.
pub fn encode_message(envelope: &Envelope, payload: &[u8]) -> Vec<u8>;

/// Decode an envelope from the front of a frame payload buffer.
/// Returns the envelope and the remaining bytes (message body).
pub fn decode_envelope(data: &[u8]) -> Result<(Envelope, &[u8]), DecodeError>;
```

### Golden-Byte Verification

A test in `tests/envelope_golden.rs` encodes a known envelope and asserts the exact byte output matches the architecture spec §6 bit layout:

- `wire_version` (4 bits) + `domain` (4 bits) + `msg_type` (7 bits) pack continuously LSB-first
- Then `session_id` (32 bits LE), `timestamp` (48 bits LE), `msg_id` (1-bit presence + optional 32 bits LE)

If the generated encoding doesn't match, the test fails — indicating a Vexil sub-byte packing issue that must be investigated.

---

## Priority Mapping

`priority.rs` — hand-written mapping from (domain, msg_type) to bus priority class.

### Public API

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Critical,
    Reliable,
    High,
    Normal,
    Low,
}

/// Look up the bus priority for a message by its envelope domain and type.
/// Returns None for unknown domain/type combinations.
pub const fn priority_of(domain: u8, msg_type: u8) -> Option<Priority>;
```

### Priority Assignments (from schema design spec)

| Priority | Messages |
|----------|----------|
| Critical | Input: KeyEvent, MouseEvent, SignalInput, Resize |
| Reliable | Handshake: all. Shell: CommandStarted, CommandFinished, PromptReady. Mux: all. Session: all. Task: all. Render: InitialState, SyncRequest, SlowClientDisconnect. System: StructuredOutput, Error |
| High | Render: RenderBatch |
| Normal | Shell: OutputChunk. Render: FrameAck, ScrollbackRequest, ScrollbackResponse |
| Low | System: PluginEvent, Diagnostic, Heartbeat |

The `priority_of` function is a `const fn` match on `(domain, msg_type)` using the domain ID table (Handshake=0, Shell=1, Input=2, Mux=3, Session=4, Task=5, Render=6, System=7) and the `@type(0xNN)` values from each schema.

### Future: Codegen Replacement

When vexil-lang ships custom annotation support (Gap 2), this hand-written table will be replaced by codegen-emitted constants. The `priority_of` function signature stays the same — only the implementation changes.

---

## Domain Module Pattern

Each domain module follows the same pattern:

```rust
// src/shell.rs
#![allow(clippy::all)]  // Generated code may trigger clippy

include!(concat!(env!("OUT_DIR"), "/malt.shell.rs"));
```

Some modules may add hand-written helpers after the include (e.g., convenience constructors, From impls). Most will be just the include.

The `lib.rs` re-exports:

```rust
pub mod framing;
pub mod envelope;
pub mod priority;
pub mod common;
pub mod handshake;
pub mod shell;
pub mod input;
pub mod mux;
pub mod session;
pub mod task;
pub mod render;
pub mod frame_element;
pub mod system;
pub mod elevate;
pub mod persist;
```

Consumer usage: `use malt_protocol::shell::OutputChunk;` or `use malt_protocol::common::PaneId;`.

---

## Testing Strategy

### 1. Envelope Golden-Byte Test

Encode a known `Envelope`:
```
wire_version=1, domain=1, msg_type=1, session_id=42,
timestamp=1000000, msg_id=Some(99)
```

Assert the exact hex output matches hand-computed expected bytes from the architecture spec bit layout. This is the critical correctness gate — it validates that Vexil sub-byte packing produces the expected wire format.

### 2. Framing Roundtrip Tests

- Empty payload: write frame, read back, assert equal
- Max-size payload (64 KiB): roundtrip
- Continuation flag set: roundtrip
- All flag combinations: roundtrip
- Frame exceeding max size: write succeeds, read returns `FrameTooLarge`
- Truncated frame: read returns `UnexpectedEof`
- Reserved flags set: read returns `ReservedFlagsSet`

### 3. Priority Mapping Test

Exhaustive test: for every (domain, msg_type) pair defined in the schema design spec, assert `priority_of` returns the correct priority. If a new message type is added to the schemas without updating the priority table, this test must fail (enforced by a count assertion — "N messages mapped, N expected").

### 4. Message Roundtrip Tests

For each domain, encode a representative message with all fields populated, decode it, assert equality. At minimum:
- `shell::CommandStarted` (simple message)
- `render::RenderCommand::DrawText` (union variant with nested `ResolvedStyle`)
- `common::LayoutNode::Split` (recursive type)
- `frame_element::FrameElement::Custom` (recursive optional)
- `persist::session::PersistedSession` (map field, nested types)
