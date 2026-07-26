# Brief 004 — `FrameWriter` lacks the reader's size bound

**Severity**: Medium · **Verified**: 2026-07-26 · **Source**: audit A-14

## What is wrong

`crates/malt-protocol/src/framing.rs` enforces `PROTOCOL_MAX_FRAME_SIZE`
(16 MiB) on the read path — `max_frame_size: max.min(PROTOCOL_MAX_FRAME_SIZE)`
at line 92 — but the write path does not:

```rust
let len = frame.payload.len() as u32;   // line ~150, unchecked
```

Two distinct failures:

- **Payload between 16 MiB and 4 GiB**: the cast is lossless, the frame is
  emitted, and the peer's reader **rejects it**. The daemon has produced a
  frame its own protocol forbids, and the connection dies for a reason the
  sender cannot see.
- **Payload above 4 GiB**: the cast wraps, and a frame is emitted with a
  length header that does not describe its body. The peer reads the wrong
  number of bytes and the stream is desynchronised from that point on.

## Why it matters

It is a protocol invariant enforced on one side only. Nothing currently
generates a 16 MiB frame — which is why it has not bitten — but streaming
command output (feature 006) made large payloads a normal occurrence rather
than a pathological one, and a single unbounded command output is now the
most likely source.

The wrap case is the serious one: a desynchronised stream is a
harder-to-diagnose failure than a refused connection, and it presents as
arbitrary protocol corruption.

## What done looks like

- `write_frame` returns an error for a payload exceeding the same bound the
  reader enforces, from the same constant — not a second copy of the number.
- A test writing an over-sized payload and asserting the error, plus a
  round-trip at exactly the boundary.
- Callers handle the error rather than unwrapping it (see
  [brief 001](001-hard-invariant-violations.md)).

## Gotchas

- **Use the reader's constant.** Two constants that can drift is how the
  asymmetry arose.
- Check whether any caller currently relies on writing large frames before
  turning this into a hard error — output streaming is the candidate. If one
  does, the fix is chunking at the producer, not a larger limit.
- The boundary test must use exactly `PROTOCOL_MAX_FRAME_SIZE`, since
  off-by-one at the limit is the classic error here.
