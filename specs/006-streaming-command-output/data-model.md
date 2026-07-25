# Phase 1 Data Model: Streaming Command Output

**Feature**: `specs/006-streaming-command-output/`
**Date**: 2026-07-25

---

## OutputChunk

A piece of a command's output, as produced.

**This message already exists** — `schemas/shell.vexil:37`,
`@domain(Shell) @type(0x04)`, with `MSG_OUTPUT_CHUNK` already in
`crates/malt-protocol/src/codec.rs:30`. It currently carries `data: bytes` and
`command_tag: optional<string>`, and its doc comment says MASH sets the
command association "at emission time". It was designed for this and never
wired. The table below is the *target* shape; `sequence` and `stream` are the
additions. See `contracts/output-chunk-vnp.md`.

| Field | Type | Notes |
|---|---|---|
| `sequence` | `u64` | Monotonic per session, never reused. This is what a consumer resumes from. |
| `command_id` | `u32` | The execution this belongs to. Ties a chunk to the same id `history` and lifecycle events use (FR-013). |
| `stream` | `Stdout \| Stderr` | FR-004. Kept as a field rather than two channels so ordering between them is preserved. |
| `data` | `Vec<u8>` | Raw bytes. **Never** a `String`: output may be invalid UTF-8, and a character may be split across chunks (FR-011). |
| `produced_at` | `u64` | Epoch ms, consistent with `CommandBlock` and lifecycle events. |

**Validation / invariants**

- `sequence` is assigned by the session's control actor, not the worker, so a
  single writer defines the order.
- Chunk boundaries carry no meaning. A consumer must not assume a chunk is a
  line, a write, or a complete character. Concatenating a command's chunks in
  sequence order reproduces its output exactly (SC-006).
- Empty `data` is not emitted. A zero-byte write is not an event.

---

## OutputLog (per session)

Bounded retention of recent chunks, and the source for resumption.

| Field | Type | Notes |
|---|---|---|
| `chunks` | ring of `OutputChunk` | Bounded by **total bytes**, not chunk count — a count bound does not bound memory when chunk sizes vary (FR-012, SC-004). |
| `next_sequence` | `u64` | Monotonic. |
| `subscribers` | `Vec<OutputSubscriber>` | |

**Behaviour**

- Appending past the byte bound evicts oldest-first.
- A resume request for a sequence older than the oldest retained chunk is
  answered with a gap naming what was lost, then the stream continues — never
  silently starting late (FR-009).
- Retention is in-memory and non-durable. A daemon restart loses it; history
  and lifecycle events remain the durable record.

**Relationship to existing state.** The log is *not* the terminal grid. The
grid (`malt-compat`) is a rendered screen with its own bounded scrollback;
the log is a byte stream with sequence numbers. Both are fed from the same
chunks, which is what keeps them consistent (FR-013).

---

## OutputSubscriber

A consumer receiving chunks as they are produced.

| Field | Type | Notes |
|---|---|---|
| `id` | `u64` | |
| `last_sent` | `u64` | Sequence high-water mark; set on resume so a resuming subscriber's gap is reported from the right position. |
| `tx` | bounded sender | Capacity `N`, allocated `N + 1`. |

**The reserved slot is load-bearing.** The channel is allocated one slot
larger than its nominal capacity and that slot belongs to the terminal gap
notification. Without it, a subscriber that overflows cannot be *told* it
overflowed — the notification's own send fails on the buffer that just
filled — and it is dropped silently. This exact defect occurred in feature
004 and was caught only because a test asserted the notification arrived.
See `crates/malt-daemon/src/executor/events.rs`.

**Delivery outcomes**: `Delivered` | `Lagged` | `Closed`. A lagged subscriber
receives a gap and is dropped; it is never accommodated by growing its buffer
(FR-010, SC-005).

---

## OutputSink (in `mash`)

The seam that lets a command's output leave `mash` before it ends.

- An optional handle on `Env`, consulted **only** for the top-level command's
  stdout/stderr when not redirected and not part of a pipeline.
- **Must be cleared** for command substitution and any capturing subshell.
  `Env::clone()` is used for those, so an inherited sink would stream
  `$(cmd)`'s output into the session *and* corrupt the substitution value.
  This is the single highest-risk invariant in the feature; Smoosh covers it.
- Writes are `&[u8]`. `mash` defines the trait; the daemon implements it.
  `mash` must not learn what a session is (Principle VII).

---

## State transitions

```
command starts        → ExecutionStarted (existing, unchanged)
output produced       → OutputChunk … OutputChunk        (NEW, repeatable)
command ends          → ExecutionCompleted (existing)
                        + command_finished event (existing)
```

Chunks are ordered *before* completion on the same channel, so a command can
never report finishing before its last output is delivered (R4). A consumer
that sees `command_finished` for a command id has already seen every chunk
for it.
