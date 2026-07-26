# Implementation Plan: A Privileged Helper That Performs Privileged Operations

**Branch**: `008-privileged-helper` | **Date**: 2026-07-26 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/008-privileged-helper/spec.md`

## Summary

Make `malt-elevate` a privileged helper that performs privileged operations
or refuses honestly, and route one container operation through it end to end.

Phase 0 changed the shape of this work in one important way: **the helper has
no server.** `main.rs` loads a nonce, prints two lines, and exits; nothing
calls `dispatch_request`, including its own binary. The nine
success-returning stubs are real and must be fixed, but they are currently
unreachable — so the bulk of the work is building the transport and the
privileged host, not adapting them.

Three further findings shape the design: the schema exists but is compiled by
nothing (Constitution V unmet on this channel); the nonce is a replayable
bearer secret described by two comments as single-use and rotating; and the
request union carries no session identity while accepting arbitrary pids and
paths — so FR-012 requires a schema change, not just a check.

## Technical Context

**Language/Version**: Rust, edition 2021 (workspace `rust-version`)

**Primary Dependencies**: `windows-sys` 0.61 (adding `Win32_System_Services`,
`Win32_System_Pipes`, `Win32_Security` for service host, named pipe, and peer
identity); `vexil-lang` at build time to compile `schemas/elevate.vexil`;
existing `malt-protocol` framing; `thiserror`, `tracing`

**Storage**: Filesystem only — the service's installed state is OS-registered;
no MALT-owned persistence is added by this feature

**Testing**: `cargo test`; boundary-crossing tests that cannot run on the host
skip loudly with a reason (see R8)

**Target Platform**: Windows is the only platform gaining a privileged
context. Linux and macOS report the helper unavailable and refuse dependent
operations with that reason (spec Assumptions)

**Project Type**: Multi-crate Rust workspace; this feature touches
`malt-elevate` (standalone binary, outside the layer system), `malt-platform`
(L0), `malt-daemon` (L2), and `mash` (L1) for the isolation-carrier
consolidation

**Performance Goals**: Not a performance feature. One constraint only: a
helper operation must not block a session's command dispatch indefinitely —
requests carry a bounded timeout, and a timeout is reported as indeterminate
(FR-005), never as success

**Constraints**: The helper runs with privilege the caller lacks; every design
choice is subordinate to it not becoming a local privilege-escalation
primitive (FR-010 to FR-013)

**Scale/Scope**: Ten operations defined; one (`ManageHcsContainer`)
implemented for real, one (`CreateSymlink`) corrected, eight made to refuse
honestly

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| **I. VT codes confined** | ✅ N/A | No VT handling in this feature |
| **II. OS calls confined** | ⚠️ **Pre-existing violation, fixed here** | `dispatch.rs:142` calls `std::os::unix::fs::symlink` directly. `malt_platform::fs::create_symlink` already exists (`fs.rs:282`). Service, pipe, and peer-identity calls are **new OS surface and MUST live in `malt-platform`**, not in `malt-elevate` — see Complexity Tracking |
| **III. Dependency-free foundations** | ✅ | `malt-protocol` and `malt-plugin-sdk` untouched |
| **IV. Safety is explicit** | ⚠️ **Action required** | `auth.rs:30` has `.expect("length checked above")` in non-test code. New `unsafe` in service/pipe code needs `// SAFETY:` |
| **V. VNP only** | ⚠️ **Currently unmet, fixed here** | The channel uses hand-rolled tag bytes (`protocol.rs:18-29`) while `schemas/elevate.vexil` sits uncompiled. Compile it and use generated types |
| **VI. Smoosh gate** | ✅ **Does not apply** | Neither `mash` nor `malt-tools` behaviour changes. `mash/src/env.rs` is touched only to *remove* a field (R6), which cannot alter shell semantics |
| **VII. Layer violations** | ✅ | `malt-elevate` sits outside the layer system and depends downward only |
| **VIII. Vendor, never depend** | ✅ | vexil-v2's service is **reference for shape only**. No path or git dependency. Nothing is copied without being rewritten to these invariants and given a real-OS test |
| **IX. No silent scope-jumps** | ✅ | Images, Linux/macOS backends, checkpointing and the remaining eight operations are named out of scope in the spec |
| **X. Commit at checkpoints** | ✅ | Each user story is a commit boundary; US2 is large enough to commit in parts |

**Verdict: PASS with three required corrections** (II, IV, V), each of which
is a pre-existing defect this feature must not build on top of. None is a
justified violation; all are fixed.

## Project Structure

### Documentation (this feature)

```text
specs/008-privileged-helper/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── elevate-protocol.md
├── checklists/
│   └── requirements.md
└── tasks.md             # Created by /speckit-tasks
```

### Source Code (repository root)

```text
schemas/
└── elevate.vexil                    # MODIFIED — session identity in the
                                     #   envelope; ManageHcsContainer typed

crates/malt-platform/                # L0 — all new OS surface lands here
└── src/
    ├── service/                     # NEW — Windows service registration
    │   ├── mod.rs                   #   install / uninstall / status
    │   └── windows.rs               #   SCM calls, all `// SAFETY:` documented
    ├── ipc/                         # NEW — named pipe + peer identity
    │   ├── mod.rs
    │   └── windows.rs               #   pipe server, client, peer token query
    ├── fs.rs                        # (existing create_symlink — now used)
    └── isolation/
        └── tier.rs                  # MODIFIED — IsolationContext gains the
                                     #   ability to carry a container identity

crates/malt-elevate/                 # Standalone binary — logic only, no OS calls
├── build.rs                         # NEW — compiles schemas/elevate.vexil
└── src/
    ├── main.rs                      # REWRITTEN — real serve loop
    ├── server.rs                    # NEW — accept, authenticate, dispatch
    ├── auth.rs                      # REWRITTEN — peer identity + anti-replay
    ├── dispatch.rs                  # MODIFIED — stubs refuse; symlink via
                                     #   malt-platform; HCS implemented
    ├── capability.rs                # NEW — what this host can actually do
    └── protocol.rs                  # REPLACED by generated types

crates/malt-daemon/src/
├── elevate_client.rs                # NEW — daemon side of the channel
└── executor/session_thread.rs       # MODIFIED — isolation carrier

crates/mash/src/env.rs               # MODIFIED — one carrier, not two
crates/malt-bin/src/                 # MODIFIED — elevate install/status/uninstall
```

**Structure Decision**: New OS surface (service control, named pipes, peer
token identity) goes in **`malt-platform`**, not in `malt-elevate`, even
though `malt-elevate` is its only consumer today. Constitution II admits no
exception for "the crate that needs it", and R4 shows what happens when this
crate is allowed to make its own OS calls: it grew a worse duplicate of
`malt_platform::fs::create_symlink` and a live `std::os::unix` violation.
`malt-elevate` keeps authentication policy, dispatch, validation and
capability reporting — the security logic — and calls down for every syscall.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|---|---|---|
| Two new `malt-platform` modules (`service/`, `ipc/`) with a single consumer | Constitution II confines OS calls to `malt-platform`, and this feature adds service-control, named-pipe and peer-token calls | Putting them in `malt-elevate` is simpler and is exactly how `dispatch.rs:142`'s `std::os::unix` violation arose. The rule has already been tested here and lost |
| A schema change (session identity) inside a feature framed as "make the stubs honest" | FR-012 cannot be satisfied without it: no request variant or envelope carries a session, so the helper has nothing to scope against (R5) | Validating against the connection's identity alone was considered. Rejected: it authenticates the *daemon*, not the session, so one user's daemon could still act on another session's pid or path |

## Phase 1 outputs

- [data-model.md](./data-model.md) — the entities, and what changes shape
- [contracts/elevate-protocol.md](./contracts/elevate-protocol.md) — schema,
  operation contract, capability surface, CLI surface
- [quickstart.md](./quickstart.md) — runnable validation, one scenario per
  success criterion

## Post-Design Constitution Re-check

| Principle | Status after design |
|---|---|
| **II. OS calls confined** | ✅ All new OS surface is in `malt-platform::{service,ipc}`; `malt-elevate` calls down. The `std::os::unix` violation is removed |
| **IV. Safety explicit** | ✅ `auth.rs:30`'s non-test `expect` is removed in the rewrite; the contract requires `// SAFETY:` on every new `unsafe` |
| **V. VNP only** | ✅ `build.rs` compiles `elevate.vexil`; hand-rolled `MessageTag` is deleted rather than left beside the generated types |
| **VIII. Vendor never depend** | ✅ Reference only; no dependency added. Every ported idea gets a real-OS test before it gets a caller |
| **IX. No silent scope-jumps** | ✅ The schema change (R5) is recorded here and in the contract as a deliberate, spec-driven change, not absorbed silently |

**Re-check verdict: PASS.**

---

## MALT standing rules for plans

*Appended by the `malt` preset. Not part of this feature.*

### Phase 0 verifies against code, with `file:line`

Not against the architecture document, not against a prior audit, not against
this repo's own backlog. Several audit findings have turned out partly stale
when checked, and one was already closed. **A research note that restates an
unverified claim is worse than none, because it reads as confirmation.**

### Survey before designing anything new

The most common defect in this repo is not a missing mechanism — it is a
mechanism that exists, is tested, and is called by nothing. Five instances so
far: Gateway auth, `AuthorityTracker`, `InputClaim`/`InputAuthorityChanged`,
`OutputChunk`, and 12 of 14 `malt-platform::isolation` modules.

Before designing a component, in this order:

1. Does it exist? Search **schemas and codec constants**, not just function
   names — `OutputChunk` was found only by reading `schemas/`, with its type
   constant already allocated.
2. Is it called from production code? Grep for callers, **excluding the
   defining crate and all test files.** This is the step that gets skipped.
3. If it has a caller, is that caller reachable? `AttachClient`'s only sender
   was a test, which made the tracker look wired for months.
4. Do its tests exercise real behaviour, or construct types? Test *count* is
   not function.

### Keep corrections; do not silently swap conclusions

When a first answer turns out wrong, record the wrong answer and why it was
wrong alongside the right one. The wrong answer is usually what a shallower
look produces, so it is the more useful half for the next reader.

### Name this feature's specific failure mode

Every feature has one, and it differs. Past examples: a delivery path verified
by injecting values into it while nothing real reached it; a test asserting a
containment call returned `Ok` and concluding containment exists; controls
that did not exercise the path they claimed to.

Write it into the plan so the tasks can be shaped against it.

### State which gates apply — including when one does not

Smoosh POSIX conformance (183/3 native Windows) is a **gate**, not a note,
whenever `mash` or `malt-tools` changes. When it does not apply, say so
explicitly, so its absence is not read as an oversight.
