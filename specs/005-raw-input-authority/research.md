# Research: Authenticated Raw Input with Input Authority

**Feature**: 005-raw-input-authority | **Date**: 2026-07-25

Two things dominated this phase: how a client proves identity on a transport
that currently checks nothing, and how input reaches a command that is
already blocked reading. The second turned out to be far cheaper than
expected, and the first turned out to have a dependency the spec did not
anticipate.

## R1: Identity on the VNP transport — token in the handshake now, peer credentials later

**Decision**: Require the same bearer token the Gateway already uses, presented during the VNP handshake, before the session inventory is disclosed or any session-affecting frame is accepted. Keep the check behind a transport-neutral seam so moving to peer-credential authentication later is a swap, not a redesign.

**Rationale**:

- It closes the hole now, over the transport that exists. The VNP listener binds raw loopback TCP, which carries no peer credentials at all — there is nothing to authenticate *with* short of changing transports.
- It reuses the Gateway's `TokenStore` and `AuthScope` rather than inventing a second identity model, which the spec's Assumptions require ("a client's rights do not depend on which door it came through").
- The trust boundary is unchanged and honest: the token file is readable by the owning user, so this authenticates "a process running as the user who started the daemon." That is exactly the boundary the Gateway already asserts. It is not weaker than what HTTP clients get today — it is the same.

**Deferred, and recorded rather than pretended away**: `architecture.md` specifies peer-credential identification for local connections ("Local connections are identified by PID (via peer credentials)"). That is strictly better — filesystem permissions gate access and there is no token to leak — and `malt_platform::sockets::Transport` *already models* `UnixSocket` and `NamedPipe` with a `default_local()` that picks correctly per platform. The VNP listener simply does not use it. Migrating is its own change: it touches the listener, `malt-tui`, the port-based CLI surface, and every VNP test, and peer-credential retrieval differs per platform. Backlogged.

**A dependency the spec did not anticipate.** Authenticating with the *current* token is authenticating with a guessable one: audit A-03 found `generate_random_token` derives its value from epoch nanoseconds and fixed arithmetic rather than a CSPRNG. Building VNP authentication on top of that produces a lock whose key can be recomputed by anyone who can approximate daemon start time. **A-03 is therefore in scope for User Story 1**, not a parallel cleanup — the story's own acceptance scenario ("a client that cannot prove its identity is refused") is not honestly met while identity can be forged. The same task fixes the ignored token-file write errors and stops printing the token to stdout.

**Alternatives considered**: *Switch transports first, then authenticate* — rejected as the first move: it delays closing a Critical hole behind a migration, and the migration is safer once the authentication seam exists to move. *Trust loopback because only local processes can connect* — rejected outright; that is the current behavior and the finding.

## R2: Raw input delivery — a session pipe registered at fd 0, which `read` already consults

**Decision**: Give each session a pipe. Register its read end at fd 0 in the session's `mash::Env`; the session's control actor holds the write end and writes accepted client input into it.

**Rationale**: This is the cheap path, and it exists already. `mash`'s `read` builtin resolves its source as:

```rust
stdin_file.or_else(|| env.open_fd_read(0).ok())    // registered fd 0
// ... only if that yields nothing:
let stdin = std::io::stdin();                       // the daemon's own console
```

So registering fd 0 makes `read` take input from the session **without modifying the builtin at all**. The fall-through to the daemon's console — the behavior audit A-07 flags and FR-008 forbids — simply stops being reachable once fd 0 is present. Everything needed is already in the workspace: `malt_platform::io::create_pipe()` is cross-platform, and `Env::register_fd(0, file)` / `SharedFdRegistry::register_file_at` place a file at a specific descriptor number.

**External processes need a second, separable step.** mash spawns external commands with `Io::Inherit` for stdin (`executor.rs` ~1284, ~5682, ~5895), so a REPL or an `ssh` password prompt would inherit the *daemon's* stdin rather than the session pipe. `Io::File(std::fs::File)` already exists as a variant, so the change is to pass a handle to the session's fd 0 instead of inheriting when one is registered. Builtin `read` and external processes are therefore two tasks, not one — and the spec's story ("a REPL, an installer") needs both to be true.

**Alternatives considered**: *Change the `read` builtin to consult a new session-input API* — rejected: it invents a second mechanism beside the fd table `read` already honors, and would leave every other fd-0 consumer still falling through to the daemon console. *Route input through a PTY* — rejected for this feature: sessions run an in-process shell, not a PTY-backed child; PTY-based input is the compat-pane path and a separate concern.

## R3: Byte-for-byte delivery — the current path corrupts three ways

**Decision**: Carry input as bytes end to end and write them to the pipe unmodified. Do not decode, do not trim, do not drop empties.

**Rationale**: `WriteInput` today does `String::from_utf8_lossy(&data)`, then `.trim()`, then submits the result as a *command*. Each step independently breaks FR-009: lossy decoding replaces invalid bytes with U+FFFD (fatal for a binary-ish payload), trimming destroys leading/trailing whitespace (which a password may contain and which a REPL may treat as significant), and the empty-check silently discards a bare newline — precisely the byte a confirmation prompt is waiting for.

## R4: Attribution — `client_id` on the input commands

**Decision**: Add the authenticated client's identity to the input-carrying `SessionCommand` variants (`KeyInput`, `WriteInput`, and the new raw-input path), and to the Gateway's input entry point.

**Rationale**: `SessionCommand::KeyInput { key }` carries no client identity, so there is structurally nothing to check authority against — the audit calls this out as the decisive detail, and FR-006 cannot be satisfied without it. The VNP listener already allocates a `client_id` per connection (it is passed to `RegisterVnpClient`); it is simply not attached to input. Once R1 binds that id to an authenticated principal, threading it through is mechanical.

## R5: Authority — the tracker exists and is correct; it is unreachable

**Decision**: Wire the existing `AuthorityTracker` to the real attach/detach path rather than writing a new one.

**Rationale**: `crates/malt-daemon/src/connection/authority.rs` already implements `attach`, `detach`, `claim`, `holder`, and `attached_clients`, and is unit-tested. Its problem is reachability, not correctness: the only commands that touch it (`AttachClient`/`DetachClient`) have zero production call sites, because the real VNP attach path goes through `RegisterVnpClient`, which never informs it. `InputAuthority` already has `Exclusive`/`Shared`/`Observe` variants. The work is to make `RegisterVnpClient`/`UnregisterVnpClient` drive the tracker, honor the wire `AttachSession.authority` field that `wait_for_attach` currently parses and discards, and add claim/notify frames.

**Consequence worth stating**: this feature will delete the "designed but never wired" status of `AuthorityTracker` — but only if the tests drive attach → claim → reject through the real VNP path rather than calling the tracker directly. A test that exercises `AuthorityTracker` in isolation would pass today, and proves nothing.

## R6: Retained input must be bounded, and is not durable

**Decision**: Bound the type-ahead a session will hold and refuse beyond it with a clear error. Do not persist it across restart.

**Rationale**: FR-012 requires a bound; an OS pipe already supplies one (writes block when the buffer fills), but blocking is exactly what the control actor must never do — the same constraint features 002 and 004 established. So the write must be non-blocking with an explicit refusal when the pipe is full, mirroring the `try_send`-and-report pattern `events.rs` uses. On durability: the spec's assumption stands — replaying type-ahead into an unrelated command after a restart is worse than losing it.

## R7: Confidentiality — the two surfaces built in 003 and 004

**Decision**: Raw input goes to the pipe and nowhere else. It must not enter the command history ring buffer, and must not be published as a lifecycle event.

**Rationale**: FR-010 exists because those two surfaces now exist and both record command text at `Read` scope. Today's `WriteInput` submits input as a command, which means it acquires a `command_id`, gets a `CommandBlock`, is persisted, and is published as `CommandStarted` with its text — a password would land in a durable file and a live event stream. The fix is structural rather than a filter: raw input never reaches `run_mash_command`, so there is no path to either surface. A test must assert the absence directly, because "we didn't call that function" is not something a reader can verify later.

## R8: Sequencing and merge checkpoints

**Decision**: Four checkpoints, in story order: authenticated identity (US1, including the A-03 token fix), raw input to builtin `read` then to external processes (US2), authority arbitration (US3), handover and lifecycle (US4).

**Rationale**: US1 first is not merely the spec's ordering — it is a safety property. Shipping US2 before US1 would make password prompts injectable by any local process, which is strictly worse than today's inability to answer them at all. Each subsequent story is independently testable and independently mergeable, which suits the merge-at-green-checkpoints workflow.
