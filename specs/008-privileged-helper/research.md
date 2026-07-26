# Phase 0 Research: A Privileged Helper That Performs Privileged Operations

**Feature**: `specs/008-privileged-helper/`
**Date**: 2026-07-26

Every finding below was checked against code with `file:line`, not against
`docs/design/architecture.md`, the backlog, or the feature description. Two of
the spec's own premises turned out understated, and one turned out to describe
a smaller problem than exists.

---

## R1. The helper does not listen. There is no server at all.

**Finding.** `crates/malt-elevate/src/main.rs:91-105` is the whole binary:
parse args, load the nonce, print two lines, return `SUCCESS`.

```rust
// Phase 2: print status and exit. Full IPC loop comes in Phase 3.
eprintln!("malt-elevate: nonce loaded from {:?}, socket {:?}", ...);
eprintln!("malt-elevate: Phase 2 skeleton — IPC loop not yet implemented");
```

There is no listener, no accept loop, and no call to `dispatch_request` from
anywhere — **including from its own binary**. `--socket` is parsed, validated
as present, and then never used for anything.

**This corrects the feature description.** The spec says nine operations
"dispatch to `stub_success`", which is true of `dispatch::dispatch_request`
(`dispatch.rs:78-93`). But that function is a library entry point with zero
callers in the workspace. The fail-open is real and must still be fixed, and
it is *currently unreachable* — not because anything guards it, but because
nothing ever gets that far.

**Consequence for the plan.** User Story 2 is not "make an existing transport
privileged". It is "there is no transport". The plan must budget for building
the server, not adapting one. This is the single largest correction Phase 0
produced, and it moves work from US1 (small) to US2 (large).

**Why the original reading was reasonable, and kept:** `dispatch.rs` has a
complete request enum, a dispatcher, error handling and eleven tests. It looks
like a working component whose handlers are pending. Reading it alone, the
conclusion "the operations are stubbed" is exactly right; it is only wrong
about what that implies.

---

## R2. The schema exists, is complete, and is compiled by nothing

**Finding.** `schemas/elevate.vexil` defines all six messages and all ten
operations, versioned `0.1.0`. Nothing compiles it: no `build.rs` references
it, and `crates/malt-elevate/Cargo.toml` has no build dependency on
`vexil-lang` (deps are `malt-protocol`, `malt-platform`, `thiserror`,
`tracing`).

`dispatch.rs:8` says its enum variants "mirror the `ElevateRequest` union from
`elevate.vexil`" — a hand-maintained copy of a schema that already exists, kept
in sync by nobody. `protocol.rs:7-10` states the same intent openly: "Full
encode/decode via vexilc-generated Pack/Unpack traits will replace the
hand-written serialization when the codegen pipeline is integrated."

**Constitution V** ("VNP is the only inter-component protocol — no ad-hoc JSON
side-channels between components that are supposed to be behind the protocol
boundary") is therefore **not satisfied on this channel**. Hand-rolled tag
bytes in `protocol.rs:18-29` are the current wire format.

**Decision.** Compile `elevate.vexil` and use generated types. Building a
transport on the hand-written mirror would entrench a second encoding at
exactly the boundary the constitution names — and the schema is already
written, so this is wiring, not design.

---

## R3. Two documents assert a nonce lifecycle that does not exist

**Finding.** The security property the design leans on is stated twice and
implemented zero times.

| Where | Claim |
|---|---|
| `schemas/elevate.vexil:8` | "Single-use nonce from the nonce file. Rotated hourly with 30s overlap." |
| `auth.rs:44-45` | "the nonce is single-use and rotated hourly" — given as the reason constant-time comparison is not strictly necessary |

`NonceAuth` (`auth.rs:14-53`) holds one `u64` and compares it. There is no
rotation, no expiry, no single-use tracking, and **nothing anywhere writes the
nonce file** — no producer exists in the workspace.

So `validate()` accepts the same value indefinitely. **FR-011 (reject replayed
requests) is not partially met; it is entirely unmet, behind a comment saying
it is handled.**

**Note the shape.** This is the same defect class as `hcs.rs`'s "synchronous
start pattern used throughout this module" comment fixed earlier today: a
comment asserting a convention the code does not implement, which is why the
gap survived review. Worth stating in the plan so the tasks treat comments as
claims to verify, not as documentation.

**Decision.** The nonce as designed proves *read access to a file*, not the
identity of a process. Against the threat FR-010/FR-012 name — any local
process on a machine-wide helper — a bearer secret in a file is weak, and a
replayable one is weaker. The design phase must choose an OS-level caller
identity mechanism as the primary control, with any shared secret as
defence in depth rather than the whole of it.

---

## R4. The one "real" operation violates Constitution II and duplicates existing code

**Finding.** `CreateSymlink` is the single non-stub. Its Unix path
(`dispatch.rs:141-146`) calls `std::os::unix::fs::symlink` directly.

Constitution II: *"No `nix`, `windows-sys`, `libc`, or `std::os::unix` outside
`malt-platform`."* `std::os::unix` is named explicitly. **This is a live
violation**, in the crate that is meant to be the most security-sensitive in
the workspace.

Worse, it is unnecessary: **`malt_platform::fs::create_symlink` already
exists** (`crates/malt-platform/src/fs.rs:282`), with Windows file-vs-directory
flag handling (`fs.rs:295-345`) more careful than the helper's own
(`dispatch.rs:120-138`).

**Decision.** Route through `malt_platform::fs::create_symlink`. This is the
survey lesson recurring inside this feature's own scope: the mechanism existed,
was tested, and someone rebuilt a worse copy beside it.

---

## R5. The protocol has no session identity, so FR-012 cannot be satisfied as designed

**Finding.** Reading the `ElevateRequest` union in `schemas/elevate.vexil`,
**no variant carries a session identifier**, and neither does
`ElevateRequestEnvelope` (which has only `request_id` and `request`).

What the variants do carry is caller-chosen authority:

| Operation | Caller supplies |
|---|---|
| `MountOverlay` | three arbitrary filesystem paths |
| `CreateSymlink` | arbitrary target and link paths |
| `BindPort` | arbitrary port and socket path |
| `SetCgroup`, `CreateNamespace`, `ApplySeccomp`, `SetupNetns`, `ApplySeatbelt`, `CreateRestrictedToken` | an arbitrary `pid` |
| `ManageHcsContainer` | an `operation` string and an opaque `config : bytes` |

A privileged process that accepts an arbitrary pid and an arbitrary path from
an unauthenticated-by-identity caller is a local privilege escalation
primitive, not a helper.

`ManageHcsContainer` is the sharpest case and the one this feature must
implement: `config : bytes` is a document handed straight to a privileged
container API. That is FR-013's "unvalidated parameters to a privileged OS
call", by construction rather than by oversight.

**Decision.** The envelope gains a session identity, and every operation is
validated against what that session is entitled to — pids must belong to it,
paths must fall inside its own storage, the container document must be
constructed by the helper from typed fields rather than passed through. This
is a **schema change**, so it must be settled in Phase 1 rather than
discovered during implementation.

**Alternative considered.** *Trust the daemon and validate nothing, on the
grounds that only the daemon can authenticate.* Rejected: it makes the nonce
file the only thing between any local process and arbitrary privileged
filesystem writes, and R3 shows the nonce is a replayable bearer secret. It
also fails FR-012 outright for the multi-user case the spec's assumptions
accept.

---

## R6. The isolation carrier decision (FR-015 to FR-017)

**Finding**, from `docs/findings/2026-07-26-isolation-design-doc-survey.md`
and re-checked: `IsolationContext` is set at
`crates/malt-daemon/src/executor/session_thread.rs:116` and stored at
`crates/mash/src/env.rs:314`; `isolation_context()` and
`take_isolation_context()` have **zero callers**. `job_object` at
`env.rs:320` is read at `executor.rs:5683` and is what actually works.

**Decision.** `IsolationContext` survives as the carrier; `job_object` becomes
one of the things a context can resolve to. Reasons:

- It already flows through the path the architecture document specifies, so
  the plumbing exists and only the consumer is missing.
- `Arc<JobObject>` is a Windows handle. It cannot represent a container
  identity (FR-016) without becoming a different type, at which point it is
  `IsolationContext` under another name.
- `Env::clone()` already propagates both fields to subshells
  (`env.rs:373`), so subshell semantics do not change.

**This is a consolidation, not a rewrite**, and it is sequenced *before* the
container backend so the backend has one place to attach to. Deleting
`IsolationContext` as dead code would be the wrong call — it is the only
abstraction already shaped for what comes next.

---

## R7. What Windows offers for the privileged context, and what MALT has

**Finding.** MALT has **no service infrastructure**: no service dependency,
no install/uninstall path, and `windows-sys` features in
`crates/malt-platform/Cargo.toml` do not include `Win32_System_Services`.

vexil-v2 does, and its shape is worth taking as reference (not as code):
a LocalSystem service reached over a named pipe
(`\\.\pipe\vexil-container-service`), with `SERVICE_PROTOCOL_VERSION`,
install/uninstall/status/restart verbs, and an explicit SDDL grant — its
comment at `windows_container_service.rs:363` reads *"LocalSystem owns the
service process. Grant full access to SYSTEM/admins and read/write access to
authenticated local clients"*.

**Note that last clause is exactly the weakness R5 identifies**: read/write for
any authenticated local client, with per-request scoping doing the rest of the
work. Taking the service shape does not mean taking that ACL.

**Decision.** Windows service host, named-pipe transport, explicit
install/uninstall/status. Caller identity comes from the pipe connection
(the OS can attribute the peer), not from the pipe's ACL alone.

---

## R8. This feature's specific failure mode

*(Required by the plan template's standing rules — each feature has a
different one.)*

**Proving the dispatch table while proving nothing about the boundary.**

Every existing test in `malt-elevate` calls `dispatch_request` in-process,
unprivileged, and asserts on the returned value — which is how eleven passing
tests coexist with nine operations that do nothing and a binary that never
listens. `stub_operations_return_success` (`dispatch.rs:153`) is the pure form:
it asserts the fail-open.

The trap for *this* feature is that User Story 1 can be "completed" by
changing `stub_success` to return `Err`, at which point the same in-process
tests pass, the same eleven tests still run in milliseconds, and **not one
byte has crossed a privilege boundary**. The spec would read as satisfied. The
system would be no more capable and only marginally more honest.

**Therefore**: no task in this feature is complete on the strength of a test
that calls a helper function directly. US1's tests must go through the
transport once US2 exists; US2's must produce real installed/uninstalled
states; US3's must compare a direct call and a routed call in the same run
(SC-007). Where a test cannot cross the boundary on the host running it, it
must skip loudly with the reason, never pass quietly.

---

## Gates that apply

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- **Constitution II** — re-check after R4's fix; this feature both removes an
  existing violation and adds a crate that will be tempted into new ones.
- **Constitution V** — the elevate channel must use the compiled schema (R2).
- **Constitution IV** — `unsafe` in any service/pipe code needs `// SAFETY:`;
  no `unwrap()`/`expect()` outside tests. Note `auth.rs:30` currently has an
  `.expect("length checked above")` in **non-test** code; the length *is*
  checked two lines above, but the invariant admits no exceptions.

**Smoosh does not apply.** Neither `mash` nor `malt-tools` is touched by this
feature. Stated explicitly so its absence is not read as an oversight.

---

## Open questions carried into Phase 1

- **Whether `ManageHcsContainer` keeps an opaque `config : bytes` or becomes
  typed fields the helper renders into a document.** R5 argues for typed;
  the cost is that the schema must model enough of a container configuration
  to be useful. Settle in the contract.
- **Whether caller identity is the pipe peer's token, a process-identity
  check, or both.** R7 leans on the pipe; the exact mechanism is a Phase 1
  contract decision, and FR-010's acceptance does not depend on which is
  chosen — only that an unauthorised local process is refused (SC-004).
