# Contract: VNP output chunk

**Feature**: `specs/006-streaming-command-output/`

Attached clients receive output over VNP. Per Principle V this is a typed,
schema-defined message, not a side channel.

---

## Schema — extend what already exists, do not add a second

**`OutputChunk` is already defined and already has a codec constant.**
`schemas/shell.vexil:37` declares it at `@domain(Shell) @type(0x04)`, and
`MSG_OUTPUT_CHUNK = 0x04` is in `crates/malt-protocol/src/codec.rs:30`. Its
own doc comment says *"command_tag is present when output originates from a
known command (MASH sets it at emission time)"* — the message was designed
for exactly this feature and then never wired to anything.

An earlier draft of this contract proposed a new Session-domain message at
`0x08`. That was wrong, and it is worth recording why: this is the **fourth**
time in this feature family that the gap turned out to be "the mechanism
exists, nothing calls it" — after Gateway auth, `AuthorityTracker`, and the
`InputClaim`/`InputAuthorityChanged` pair in feature 005. Check for an
existing definition before adding one.

Current definition:

```
@domain(Shell) @type(0x04) @revision(1)
message OutputChunk {
    data        @0 : bytes
    command_tag @1 : optional<string>
}
```

**Required additions** (new fields, so `@revision` bumps; existing fields keep
their numbers, which is what keeps decoders compatible):

```
message OutputChunk {
    data        @0 : bytes
    command_tag @1 : optional<string>
    sequence    @2 : u64            # resumption point (data-model.md)
    stream      @3 : OutputStream   # stdout vs stderr (FR-004)
}

@non_exhaustive
enum OutputStream {
    Stdout @0
    Stderr @1
}
```

`data` is already `bytes`, which is correct and must stay that way: output may
be invalid UTF-8 and a character may be split across chunks (FR-011). VNP
bitpack carries bytes natively, so unlike the HTTP surface no encoding is
needed.

`command_tag` is `optional<string>` and already carries the "which command"
association. Whether to use it or add a numeric `command_id` matching
`CommandBlock.command_id` is an implementation decision; the requirement is
that a chunk is correlatable with the same command that history and lifecycle
events describe (FR-013). Note the existing field is deliberately optional —
its doc names startup output, background jobs, and raw compat-pane output as
cases with no command association, all of which remain valid.

---

## Delivery to attached clients

Chunks reach clients as a new `ClientMessage` variant, on the **same ordered
per-client stream** as `Render` and `AuthorityChanged`.

The single stream is deliberate and was settled in feature 005: a client must
not receive an authority change or a completion after frames that logically
precede them, and separate channels cannot promise ordering. The same holds
here — a chunk arriving after the frame that already renders it is
incoherent.

### What a client does with a chunk

A rendering client (`malt-tui`) does **not** need to interpret chunks: the
daemon feeds the same bytes to the terminal grid and dispatches
`Render` as usual, so US2 arrives through the existing render path. The chunk
message exists for clients that want the byte stream itself.

A client that ignores `OutputChunk` entirely still renders correctly. This
matters for compatibility: `maltty` and `malt-web` are frozen and must not
break.

---

## Ordering guarantee

For one command: every `OutputChunk` for command *N* is delivered before the
completion of command *N*.

This is a consequence of chunks and completion travelling one ordered channel
from worker to control actor (R4). It is what lets a consumer treat
`command_finished` as "I have seen everything" rather than having to wait an
arbitrary interval for stragglers.

---

## Backpressure

Chunk delivery to a client uses the existing non-blocking `try_send` on the
per-client channel, consistent with `Render`. A client too slow to keep up is
already handled by the renderer's lag and shed logic; this feature adds no
second mechanism and no unbounded queue.
