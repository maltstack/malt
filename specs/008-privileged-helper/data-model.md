# Data Model: A Privileged Helper That Performs Privileged Operations

**Feature**: `specs/008-privileged-helper/`

Entities are grouped by the requirement they exist to satisfy. Where an
entity already exists, its current shape and what changes are both given —
this feature's largest risk is building beside something rather than on it
(research R4).

---

## 1. Operation outcome — replaces "the call returned"

**Why it exists**: FR-001, FR-005, SC-002. Today `dispatch_request` returns
`Result<Vec<u8>, String>` (`dispatch.rs:70`), where `Ok(vec![])` means both
"performed" and "not implemented, pretended". Those must stop being the same
value.

```
@non_exhaustive
enum OperationOutcome {
    Performed    @0    # the effect exists; observable by the caller
    Refused      @1    # deliberately not done, with a reason
    Indeterminate @2   # started, outcome unknown (helper lost mid-flight)
}
```

| Field | Meaning |
|---|---|
| `outcome` | Which of the three above |
| `reason_code` | Machine-readable: `not_implemented`, `unsupported_platform`, `helper_unavailable`, `not_entitled`, `invalid_parameters`, `os_error`, `timed_out` |
| `detail` | Human-readable, names the operation |
| `payload` | Present only when `Performed` |

**Validation rules**

- `Performed` MUST NOT be produced by any code path that did not perform the
  operation. This is the FR-001 requirement stated as a type invariant.
- `Indeterminate` MUST be produced when a request was sent and no response
  was received — never `Refused` (which asserts nothing happened) and never
  `Performed`.
- A caller that receives `Indeterminate` MUST NOT treat it as either outcome.

**State transitions**: none — an outcome is terminal. A retried request is a
new request with a new correlation id, because a replayed one is refused
(FR-011).

---

## 2. Helper state — what status reports

**Why it exists**: FR-003, SC-003. The spec requires four situations to stay
distinguishable; collapsing them is what makes "isolation unavailable"
unactionable.

```
@non_exhaustive
enum HelperState {
    NotInstalled     @0
    InstalledStopped @1
    Reachable        @2
    VersionMismatch  @3
}
```

| Field | Meaning |
|---|---|
| `state` | One of the above |
| `protocol_version` | The helper's version, when known |
| `expected_version` | What this daemon speaks |
| `detail` | Why, in words — e.g. which service name was looked for |

**Validation rules**

- `VersionMismatch` is reported **before** any operation is attempted
  (FR-014), so it is a state, not an operation failure.
- `NotInstalled` and `InstalledStopped` MUST NOT collapse: the first calls
  for `malt elevate install`, the second for starting a service that already
  exists.
- No state may be inferred from a failed operation. State is queried
  directly; an operation failing because the helper is gone produces
  `Indeterminate` or `Refused{helper_unavailable}` *and* leaves the state
  query authoritative.

---

## 3. Session entitlement — the thing operations are scoped against

**Why it exists**: FR-012, FR-013, SC-006. Research R5 found that **no request
variant and no envelope carries a session identifier**, while variants accept
arbitrary pids, paths and ports. Without this entity the helper cannot answer
"is the caller allowed to ask for this?", so it is new, and it is the schema
change this feature makes.

| Field | Meaning |
|---|---|
| `session_id` | Which session the operation acts on behalf of |
| `owner` | The OS principal that owns that session |
| `storage_root` | The only directory tree paths may fall within |
| `pids` | The processes this session may name |

**Validation rules** — every one of these is a refusal, not a warning:

- The envelope's `session_id` MUST belong to the authenticated caller's
  principal. A request naming another principal's session is `Refused{not_entitled}`.
- A `pid` parameter MUST be a member of the named session's process set.
  Membership is established by the helper against the OS, **not** taken from
  the request.
- Any path parameter MUST resolve, after canonicalization and symlink
  resolution, inside `storage_root`. Canonicalization happens before the
  check, so `..` traversal and symlink escapes are refused rather than
  followed.
- A parameter that cannot be validated is `Refused{invalid_parameters}`. The
  helper never passes an unvalidated value to a privileged call (FR-013).

**Note on why the check must be helper-side**: the daemon is unprivileged and
may be compromised or impersonated; a check it performs is advice. The helper
holds the privilege, so it owns the decision.

---

## 4. Isolation carrier — consolidating two mechanisms into one

**Why it exists**: FR-015 to FR-017, SC-009. Two mechanisms exist today, six
lines apart in `crates/mash/src/env.rs`:

| Field | Line | Set by | Read by |
|---|---|---|---|
| `isolation_context: Option<IsolationContext>` | `env.rs:314` | `session_thread.rs:116` | **nothing** |
| `job_object: Option<Arc<JobObject>>` | `env.rs:320` | session setup | `executor.rs:5683` |

**Decision**: `IsolationContext` survives and gains the ability to name what
is actually established; `job_object` becomes one thing it can hold, not a
parallel field.

```
IsolationContext {
    tier         : IsolationTier      # what was granted
    mechanism    : IsolationMechanism # what provides it (already #[non_exhaustive])
    established  : Established        # the live handle, below
}

@non_exhaustive
enum Established {
    Nothing      @0
    JobObject    @1  # Arc<JobObject> — today's working path
    Container    @2  # a compute system identity — what FR-016 requires
}
```

**Validation rules**

- `mash::Env` MUST carry exactly **one** isolation field after this change
  (SC-009: the count is 1, it is 2 today).
- What a session reports and what constrains it MUST read from this single
  value (FR-017), so reporting cannot drift from reality.
- `established` MUST NOT be upgraded after session creation (FR-018). It may
  be downgraded to `Nothing` if containment is lost, which is spec 007's
  FR-017 and already specified there.
- `Env::clone()` MUST propagate the carrier to subshells, preserving today's
  behaviour at `env.rs:373`.

**Migration note**: `job_object` is removed as a field, not deprecated
alongside. Leaving both is how this situation arose.

---

## 5. Nonce → caller identity

**Why it changes**: FR-010, FR-011, SC-004, SC-005. Research R3 found the
current nonce is a bearer secret in a file, accepted indefinitely, described
by `schemas/elevate.vexil:8` and `auth.rs:44` as "single-use" and "rotated
hourly" — neither of which is implemented, and nothing writes the file.

| Concern | Now | After |
|---|---|---|
| Who is the caller? | Anyone who can read a file | The OS-attributed peer of the connection |
| Replay | Accepted forever | Refused: each request carries a correlation id valid once, within a bounded window |
| Rotation | Claimed, absent | Not relied upon — identity is not a secret |

**Validation rules**

- Caller identity MUST come from the connection, which the OS can attribute,
  rather than from anything the caller supplies.
- A shared secret MAY remain as defence in depth. It MUST NOT be the only
  control, and no comment may claim a property it does not implement — the
  two existing comments are corrected or deleted as part of this work.
- A replayed correlation id MUST be refused even from a correctly identified
  caller.

---

## Entity relationships

```
HelperState ──── queried before any ────▶ (no operation attempted on mismatch)
                 operation

Request ──carries──▶ SessionEntitlement ──validated by──▶ Helper
   │                                                        │
   └────────────────── produces ──────────────────────▶ OperationOutcome
                                                            │
IsolationContext ◀── updated only by ── Performed ──────────┘
                     outcomes
```

The one-way edge that matters: **only a `Performed` outcome may change an
`IsolationContext`.** `Refused` and `Indeterminate` leave it untouched, which
is what stops a session reporting containment it did not get — the same rule
spec 007 applies to tiers, applied here to the operations underneath them.
