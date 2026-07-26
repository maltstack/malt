# Contract: isolation policy in, isolation status out

**Feature**: `specs/007-fail-closed-isolation/`

Three surfaces request a session — CLI, HTTP, VNP. All three gain a policy
input and report a status instead of a bare tier. They must not diverge:
FR-007 requires creating, querying and listing to agree, and the way to
guarantee that is one status value read by all of them, not three separate
constructions of it.

---

## Schema (`schemas/common.vexil`)

```
@non_exhaustive
@doc("How much a caller cares whether the requested isolation is achieved.")
enum IsolationPolicy {
    Required  @0
    Preferred @1
    Disabled  @2
}

@non_exhaustive
@doc("How a reported isolation level is known to be in force.")
enum IsolationBasis {
    Verified @0
    Assumed  @1
    None     @2
}

@doc("What isolation a session actually has, versus what was asked for.")
message IsolationStatus {
    effective @0 : IsolationTier
    requested @1 : IsolationTier
    basis     @2 : IsolationBasis
    mechanism @3 : optional<string>
    detail    @4 : optional<string>
}
```

`IsolationTier` already exists (`Bare`/`Restricted`/`Capped`/`Contained`) and
is unchanged. **Check the next free `@type` in the session domain before
allocating one** — an earlier feature in this repo drafted a duplicate
message because an existing definition was not checked first.

`SessionInfo.isolation` changes from `IsolationTier` to `IsolationStatus`.
This is a **breaking wire change**: bump `@revision`, keep field numbers.

---

## HTTP

### `POST /sessions` — request gains a policy

```json
{ "name": "build", "isolation": "capped", "isolation_policy": "required" }
```

`isolation_policy` is optional. Omitted **and** a tier above the baseline
named means `required` (data-model.md, research R5).

### Responses

**Established** — 201 with a status:

```json
{ "ok": true, "data": { "id": 7,
  "isolation": { "effective": "capped", "requested": "capped",
                 "basis": "verified", "mechanism": "job-object" } } }
```

**Downgraded under `preferred`** — 201, and the downgrade appears *in this
response*; no second call is needed (FR-004):

```json
{ "ok": true, "data": { "id": 8,
  "isolation": { "effective": "bare", "requested": "contained",
                 "basis": "none",
                 "detail": "contained unavailable: computecore.dll not found" } } }
```

**Refused under `required`** — 409, **no session created**:

```json
{ "ok": false, "error": { "code": "isolation_unavailable",
  "message": "contained was required but could not be established: computecore.dll not found. Retry with isolation_policy=preferred to accept a lower level.",
  "requested": "contained", "best_available": "capped" } }
```

The message names `preferred` deliberately. A refusal that does not say how
to proceed turns a silent downgrade into a dead end — a different failure,
not a fix.

### `GET /isolation/capabilities` — before creating a session (FR-010)

`Read` scope. Reports, per tier, whether it can be provided here, by which
mechanism, and on what basis:

```json
{ "ok": true, "data": { "tiers": [
  { "tier": "capped", "available": true, "basis": "assumed",
    "mechanism": "job-object",
    "detail": "Job Objects present on all supported Windows versions; not probed" },
  { "tier": "contained", "available": false, "basis": "none",
    "reason_code": "missing_binary", "detail": "computecore.dll not found" } ] } }
```

`basis: assumed` is reported honestly rather than as `verified`. Research R2
found several probe facets return support unconditionally on their platform,
and FR-006 requires that to stay visible rather than be flattened into
"supported".

### Unchanged

`GET /sessions/{id}` and `GET /sessions` report the same `IsolationStatus`.
Same scope as today — containment status is session state, not a separate
privilege (FR-016).

---

## CLI

```
malt new --isolation <tier> [--isolation-policy required|preferred|disabled]
malt isolation capabilities
```

`malt new` prints the effective status, and prints a **visible** notice on
downgrade. A downgrade that appears only in structured output has not been
reported to a human.

---

## Compatibility

- **Breaking, deliberately.** `SessionInfo.isolation` changes shape, and a
  request naming a tier now defaults to `required`, so calls that previously
  returned an uncontained session will fail. Both belong in release notes.
- Callers naming no tier are unaffected; the baseline path does not change.
- The refusal is 409 rather than 400: the request is valid, the host cannot
  satisfy it.
