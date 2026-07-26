# Quickstart: A Privileged Helper That Performs Privileged Operations

**Feature**: `specs/008-privileged-helper/`

**Every scenario must cross the boundary, or say loudly that it could not.**

That is this feature's specific trap (research R8). Eleven tests pass today
against a helper whose binary never listens and whose operations do nothing,
because every one of them calls `dispatch_request` in-process and asserts on
the return value. A scenario that does the same proves nothing that is not
already true while the defect is fully present — and `stub_operations_return_success`
is the pure form of that: a passing test that asserts the fail-open.

A scenario that cannot run on this host **skips with its reason printed**. It
never passes quietly.

## Prerequisites

```bash
cargo build --workspace
```

Scenarios 1 and 2 need no privilege and no installation. Scenarios 3 onward
need an elevation prompt to be answerable on this machine.

---

## Scenario 1 (US1) — an unimplemented operation refuses, and says which

```bash
cargo test -p malt-elevate --test operation_outcomes -- --nocapture
```

**Expected**: every operation the helper exposes, invoked where it cannot be
performed, yields `Refused` naming the operation and a reason code. Zero yield
`Performed`.

**Expected NOT**: an empty success payload. `Ok(vec![])` currently means both
"done" and "pretended", which is why the outcome type changed.

**The check that matters**: this must enumerate operations **from the schema**,
not from a hand-written list in the test. A hand-written list silently stops
covering an operation the day one is added — and the hand-maintained mirror of
this exact union (`dispatch.rs:8`) is what research R2 found already drifting.

---

## Scenario 2 (US1) — the capability surface matches what actually happens

```bash
malt elevate status
```

Then, for each operation the capability surface reports **available**, invoke
it; for each reported **unavailable**, invoke it too.

**Expected**: available operations do not fail with `NotImplemented` or
`UnsupportedPlatform`; unavailable ones do, with the reason the surface gave.

**Expected NOT**: any operation reported available on the grounds that the
protocol can encode it (FR-002). A capability answer that disagrees with
reality is worse than none — it is what a caller uses to decide what to ask
for.

---

## Scenario 3 (US2) — all four helper states, on one host

Produce each and query status:

```bash
malt elevate status
```

**Expected**, in order: `not installed` → install → stop the service →
`installed, not running` → start → `reachable`.

Each reports **distinct guidance**, not one "unavailable" message
(FR-003, SC-003).

**The check that matters**: `reachable` must be reported only after a
round-trip response, not because the OS says the service is running. Service
bookkeeping is not evidence that anything answers.

---

## Scenario 4 (US2) — declined elevation leaves nothing

```bash
malt elevate install
```

Decline the elevation prompt.

**Expected**: the outcome says installation did not happen.

```bash
malt elevate status
```

**Expected**: `not installed` — **verified by inspecting the system for each
artefact installation would have created**, not by the install command having
printed an error (FR-008, SC-010). A partial install that reports failure is
still a partial install.

---

## Scenario 5 (US2) — an unauthorised local process is refused

With the helper installed and running, connect to it from a process that is
not the authorised daemon and send a well-formed request.

**Expected**: refused (SC-004).

**Verified by making the request**, not by reviewing the authentication
design. This is the scenario that decides whether the feature shipped a
helper or a local privilege-escalation primitive, and it is the one most
easily satisfied on paper.

---

## Scenario 6 (US2) — a replayed request is refused

Capture a valid request envelope from Scenario 5's authorised path and send it
again.

**Expected**: refused (FR-011, SC-005).

Note this **currently passes nothing**: `NonceAuth::validate` accepts the same
nonce indefinitely (`auth.rs:42-53`), while `elevate.vexil:8` and `auth.rs:44`
both describe it as single-use and hourly-rotated. Two documents assert the
property this scenario tests; neither is implemented.

---

## Scenario 7 (US2) — a request cannot reach outside its session

With the helper running, send requests that name:

- another principal's session id
- a path outside the session's `storage_root`, including via `..` and via a
  symlink pointing out of it
- a pid not belonging to the named session

**Expected**: each refused with `NotEntitled` or `InvalidParameters` (SC-006).

**The check that matters**: path validation must canonicalize *before*
checking, so a symlink escape is refused rather than followed. Test the
symlink case explicitly — it is the one a naive prefix comparison passes.

---

## Scenario 8 (US3) — the boundary changes the outcome

The core scenario. Both calls **in the same run**, on a host with the
container feature present:

```bash
cargo test -p malt-daemon --test elevate_boundary privilege_boundary_changes_the_outcome -- --ignored --nocapture
```

**Expected**: the direct call is refused for lack of privilege; the
helper-routed call is **not refused for that reason** (SC-007).

**Expected NOT**: an assertion that the routed call succeeded. A compute
system is image-backed and images are out of scope, so it may still fail for a
different reason — that is a pass. The claim is about *which* failure
disappears, and comparing both calls in one run is what makes it falsifiable
rather than a story about privilege.

If the host cannot run the container feature at all, **skip, printing why**.

---

## Scenario 9 (US3) — teardown leaves nothing

For a resource created in Scenario 8:

**Expected**: after teardown, enumerate the resource and find it absent
(SC-008).

**Not** by teardown returning success — the same rule spec 007's SC-006
applies to sessions, for the same reason.

---

## Scenario 10 (US3) — a lost helper is never a success

Kill the helper mid-operation.

**Expected**: the daemon reports `Indeterminate`, and does not resolve it to
either outcome (FR-005). The session's isolation status is **unchanged**,
because only a `Performed` outcome may update it.

---

## Scenario 11 (FR-015..017) — one carrier, not two

```bash
cargo test --workspace
```

**Expected**: `mash::Env` carries exactly one isolation field. Count is 1; it
is 2 today (`env.rs:314` and `env.rs:320`), and SC-009 is that count.

**The check that matters**: this is verified after User Story 3 lands, not
before. The requirement is not "there is one field" — it is that adding a
container backend did not need a parallel path. Checking it before the backend
exists tests nothing.

---

## Gate check before completion

```bash
cargo test --workspace
```

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo clippy -p malt-platform --features hcs --all-targets -- -D warnings
```

**Smoosh does not apply** — neither `mash` nor `malt-tools` changes behaviour
here (`env.rs` only loses a field). Stated so its absence is not read as an
oversight.

**Constitution re-checks specific to this feature**, each closing a defect
found in Phase 0:

- **II**: no `std::os::unix`, `windows-sys`, `nix` or `libc` outside
  `malt-platform` — `dispatch.rs:142` violates this today.
- **IV**: no `unwrap`/`expect` outside tests — `auth.rs:30` violates this
  today; every new `unsafe` carries `// SAFETY:`.
- **V**: the channel uses types generated from `schemas/elevate.vexil`, and
  the hand-rolled `MessageTag` is gone rather than sitting beside them.
