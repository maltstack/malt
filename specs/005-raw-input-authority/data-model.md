# Data Model: Authenticated Raw Input with Input Authority

**Feature**: 005-raw-input-authority | **Date**: 2026-07-25

## Entities

### ClientIdentity (NEW — established at VNP handshake)

The authenticated principal behind a connection. Everything else in this
feature attributes to it.

| Field | Type | Meaning |
|-------|------|---------|
| `client_id` | `u64` | Per-connection identity, already allocated by the VNP listener today; from this feature on it is only allocated *after* authentication succeeds. |
| `scope` | `AuthScope` | Reused from the Gateway (`Monitor < Read < Interact < Admin`), so a client's rights do not depend on which transport it arrived on. |

**Lifecycle**: established during the handshake, before the session inventory
is disclosed; discarded when the connection closes. Not persisted.

**State transitions**:

```
(connected) --no credential within the deadline--> REFUSED, connection closed
(connected) --invalid credential--> REFUSED, connection closed, nothing disclosed
(connected) --valid credential--> IDENTIFIED (inventory may now be sent)
IDENTIFIED  --connection closes--> released (authority released with it)
```

### SessionInputChannel (NEW — `crates/malt-daemon/src/executor/input.rs`)

One per session. The destination for raw input and the source `mash` reads
from.

| Field | Type | Meaning |
|-------|------|---------|
| `writer` | `std::fs::File` | Write end of the session pipe, held by the control actor. Written non-blocking; a full pipe is refused, never waited on. |
| *(read end)* | — | Registered at fd `0` in the session's `mash::Env` via `Env::register_fd`. Not held separately — the `Env` owns it. |

- Created with `malt_platform::io::create_pipe()` at session construction.
- `read` resolves fd 0 through `env.open_fd_read(0)` **before** falling back
  to `std::io::stdin()`, so registering it is what stops the fall-through to
  the daemon's own console (R2). The builtin itself is unchanged.
- Bounded by the OS pipe buffer. When full, the write is refused with a
  distinct error rather than blocking — the control actor must never wait on
  a client (R6).
- Not persisted; unconsumed type-ahead dies with the session (R6).

### InputSubmission (NEW — the request shape)

| Field | Type | Meaning |
|-------|------|---------|
| `client_id` | `u64` | Who sent it. Present on every input-carrying command from this feature on; today `SessionCommand::KeyInput` has no such field, which is why authority cannot be enforced (R4). |
| `data` | `Vec<u8>` | The bytes, carried and written unmodified. Never decoded, never trimmed, never dropped when empty (R3). |

**Outcome** is one of: delivered; refused because the sender does not hold
authority; refused because the retained-input bound is reached. All three are
reported — silence is not an outcome (FR-014).

### InputAuthority (EXISTS — `crates/malt-daemon/src/connection/authority.rs`)

Already implemented and unit-tested: `attach`, `detach`, `claim`, `holder`,
`attached_clients`. `InputAuthority` already has `Exclusive`/`Shared`/`Observe`.

**Nothing here needs writing — it needs connecting.** Its production call
sites are zero because the real attach path is `RegisterVnpClient`, which
never informs it, and `wait_for_attach` parses the wire `authority` field and
discards it.

**State transitions** (the tracker already models these; this feature makes
them reachable):

```
(no clients)      --client attaches--> that client HOLDS authority
HOLDS(A) + B attached --B claims--> HOLDS(B); A notified it no longer holds
HOLDS(A)          --A detaches or disconnects--> released; any attached client may claim
HOLDS(A)          --A claims again--> no-op, no notification to others (edge case)
(released)        --next client attaches--> that client HOLDS
```

The invariant FR-013 depends on: authority is released by *departure*, not
by consent, so a client that is gone can never strand the session.

## Relationships

```
VNP connection ──authenticates──> ClientIdentity
                                       │
                        attaches to    ▼
SessionExecutor ──owns──> AuthorityTracker   (at most one holder)
       │
       └──owns──> SessionInputChannel ──write end (non-blocking)
                            │
                            └──read end registered at fd 0──> mash::Env
                                                                 │
                                    ┌────────────────────────────┴────────────┐
                                    ▼                                         ▼
                            `read` builtin                        external process stdin
                        (already consults fd 0)                (Io::File instead of Io::Inherit)
```

## Validation rules

- **Identity before disclosure (FR-001, FR-002)**: the session inventory is assembled and sent *after* authentication. Today it is collected before the handshake and sent within it — that ordering must invert.
- **Bounded identification (FR-003, FR-004)**: a connection that has not identified within a deadline is closed, and concurrent unidentified connections are capped. The current listener sets its read timeout only *after* blocking handshake work, which is what makes connect-and-stall free (A-08).
- **Identifier is not authorization (FR-005)**: a named session id is checked against what the identity may reach, rather than honored because it was supplied.
- **Byte fidelity (FR-009)**: input arrives at the pipe exactly as sent. A test must send bytes that are not valid UTF-8, plus leading/trailing whitespace, plus a bare newline, and assert all three survive — those are the three ways the current path corrupts (R3).
- **Confidentiality (FR-010)**: raw input must not appear in command history or the lifecycle event stream. Structural, not filtered: raw input never reaches `run_mash_command`, which is the only producer of `CommandBlock`s and `CommandStarted` events. A test must assert absence from both surfaces directly.
- **Single writer (FR-013, FR-014)**: with two clients sending concurrently, the pipe receives bytes from exactly one. Interleaving is the failure this prevents, so the test must send distinguishable payloads from both and assert none of the non-holder's bytes appear.
- **Non-blocking (spec constraint)**: a full input pipe refuses rather than waits. A test must fill the pipe without a reader and assert the control actor still services other commands.
