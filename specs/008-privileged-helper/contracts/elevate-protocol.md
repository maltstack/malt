# Contract: the elevate channel, its operations, and its CLI surface

**Feature**: `specs/008-privileged-helper/`

Three surfaces change: the schema on the wire, the operation contract every
handler must satisfy, and the operator-facing CLI. They are specified together
because the invariant that matters — *an operation that did not happen never
reports success* — has to hold across all three or it holds nowhere.

---

## 1. Schema (`schemas/elevate.vexil`)

The schema already exists and defines all six messages and ten operations. It
is currently **compiled by nothing** (research R2), and `dispatch.rs` keeps a
hand-maintained mirror of the union. This feature compiles it and deletes the
mirror, satisfying Constitution V on this channel.

### The envelope gains a session identity

This is the schema change research R5 requires. Without it the helper has
nothing to scope requests against.

```
@doc("Wrapper carrying correlation and the session an operation acts for.")
message ElevateRequestEnvelope {
    request_id @0 : u32
    request    @1 : ElevateRequest
    session_id @2 : SessionId          # NEW — FR-012
    nonce      @3 : u64                # NEW — per-request, single-use (FR-011)
}
```

`request_id` correlates; `nonce` is what makes a captured envelope useless on
replay. They are separate fields because a retry is a legitimate new request
and must not be forced to reuse an identifier the helper has burned.

**Bump `@version`. Field numbers are not reused.**

### `ManageHcsContainer` becomes typed

Today: `ManageHcsContainer @7 { operation @0 : string  config @1 : bytes }` —
a stringly-typed verb and an opaque document handed to a privileged API. That
is FR-013's prohibited shape by construction.

```
@non_exhaustive
union ContainerOperation {
    Create   @0 { memory_limit_mb @0 : optional<u32>  hostname @1 : optional<string> }
    Start    @1 { id @0 : string }
    Terminate @2 { id @0 : string }
}

ManageHcsContainer @7 { operation @0 : ContainerOperation }
```

**The helper renders the container configuration document itself**, from these
typed fields plus the session's entitlement. The caller never supplies the
document. This is the single most important change in the contract: it is the
difference between a helper and an arbitrary-privileged-action service.

### Outcome replaces a bare result

```
@non_exhaustive
enum OutcomeKind { Performed @0  Refused @1  Indeterminate @2 }

@non_exhaustive
enum ReasonCode {
    NotImplemented @0   UnsupportedPlatform @1   HelperUnavailable @2
    NotEntitled    @3   InvalidParameters   @4   OsError           @5
    TimedOut       @6
}

message ElevateResponse {
    request_id @0 : u32
    kind       @1 : OutcomeKind
    reason     @2 : optional<ReasonCode>
    detail     @3 : optional<string>
    payload    @4 : optional<bytes>       # only when Performed
}
```

`result<bytes, string>` is replaced because `Ok(vec![])` currently means both
"done" and "pretended". A caller cannot distinguish them, which is the defect.

---

## 2. Operation contract — binding on every handler

Every handler MUST satisfy all six. A handler that cannot is not merged.

1. **Never report `Performed` without the effect existing.** Not "the call
   returned `Ok`" — the effect. Where the OS gives no confirmation, the
   handler queries for it.
2. **Refuse before acting when a parameter cannot be validated.** Paths are
   canonicalized then checked against the session's `storage_root`; pids are
   checked for membership in the session's process set *against the OS*, not
   against the request; identifiers are checked for ownership.
3. **Refuse rather than partially apply.** If an operation cannot be
   completed, anything it created is torn down before returning. A partial
   effect reported as `Refused` is a lie of the same family as a stub
   reporting success.
4. **Report unimplemented as `Refused{NotImplemented}`**, naming the
   operation. This is what the nine stubs become.
5. **Report platform-absent as `Refused{UnsupportedPlatform}`**, distinct
   from `NotImplemented` — "this build does not do it" and "this host cannot"
   need different responses from the caller.
6. **Bound every privileged call.** A call that cannot complete within the
   operation timeout yields `Indeterminate{TimedOut}`, never `Refused`.

### Per-operation disposition in this feature

| Operation | Disposition |
|---|---|
| `ManageHcsContainer` | **Implemented** (US3) — typed, helper-rendered document |
| `CreateSymlink` | **Corrected** — routed through `malt_platform::fs::create_symlink`, removing the `std::os::unix` violation at `dispatch.rs:142` and the worse duplicate at `dispatch.rs:120-138` |
| `CreateNamespace`, `MountOverlay`, `SetCgroup`, `SetupNetns`, `ApplySeccomp`, `ApplySeatbelt`, `CreateRestrictedToken`, `BindPort` | **Refuse honestly** — `NotImplemented` or `UnsupportedPlatform` as applies |

---

## 3. Capability surface

**Availability is a property of the host and this build, never of the protocol
accepting a message** (FR-002). The current code has no capability surface at
all; a caller's only way to learn what works is to try it, which with
success-returning stubs teaches the wrong answer.

```
message ElevateCapabilities {
    protocol_version @0 : u32
    operations       @1 : list<OperationCapability>
}

message OperationCapability {
    operation @0 : string        # the union variant name
    available @1 : bool
    reason    @2 : optional<ReasonCode>
    detail    @3 : optional<string>
}
```

**Cross-check that must hold**: what this reports and what requests then
succeed cannot disagree. A capability surface that disagrees with reality is
worse than none, because it is what a caller uses to decide what to ask for.
This mirrors spec 007's SC-007 and is verified the same way — by asking, then
doing.

---

## 4. Daemon-facing behaviour

- The daemon queries `HelperState` **before** attempting an operation, and on
  `VersionMismatch` attempts nothing (FR-014).
- When the helper is unavailable, a dependent operation fails naming that
  cause. It is **never** silently skipped and **never** silently downgraded
  (FR-004) — the downgrade decision belongs to spec 007's policy layer, which
  reports it, not to this channel.
- An `Indeterminate` outcome propagates as `Indeterminate`. The daemon MUST
  NOT resolve it to either success or failure on its own.
- Only a `Performed` outcome may update a session's `IsolationContext`.

---

## 5. CLI surface

```
malt elevate status
malt elevate install
malt elevate uninstall
```

`status` distinguishes all four states with distinct guidance:

```
$ malt elevate status
helper:   not installed
effect:   contained isolation is unavailable; required requests are refused
resolve:  malt elevate install   (prompts for elevation)
```

```
$ malt elevate status
helper:   installed, not running
protocol: 1 (matches this daemon)
effect:   contained isolation is unavailable until the helper is running
```

```
$ malt elevate status
helper:   reachable
protocol: 1
verified: responded to a capability query 12ms ago
```

**Reachable is reported only after a round trip.** "The service is registered
and marked running" is what the OS says about its own bookkeeping, not
evidence that anything answers — and that distinction is exactly what this
feature exists to stop eliding.

`install` and `uninstall` require explicit elevation, never occur as a side
effect of another command (FR-007), and on declined elevation leave nothing
behind and say so (FR-008).

---

## 6. Compatibility

- **Breaking, deliberately**: `ElevateResponse` changes shape and the envelope
  gains two required fields. `@version` is bumped; field numbers are not
  reused.
- **No external callers exist** — nothing outside `malt-elevate` references it
  (verified: only a doc-comment mention in `hcs.rs:270`). The practical
  migration is the crate's own tests, and `stub_operations_return_success`
  is **inverted, not deleted**, so the old behaviour cannot return unnoticed
  (spec US1 acceptance 3).
- **`protocol.rs`'s hand-rolled `MessageTag` is deleted**, not left beside the
  generated types. Two encodings for one channel is how they drift.
