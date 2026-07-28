# Data Model: Gateway Hardening

**Feature**: `specs/010-gateway-hardening/`

Three entities. Each replaces something that exists, so the "before" is given
alongside — the point is what changes and why the current shape cannot be
extended into the new one.

---

## 1. Allowance — what a caller may spend, and when it returns

**Why it changes**: today's state is `HashMap<String, usize>` — a count with
no clock (`rate_limit.rs:8`). Nothing in that structure can express "and it
comes back at time T", which is FR-001. `refill()` exists to reset it and has
zero production callers, which is why the refusal is permanent.

| Field | Meaning |
|---|---|
| `used` | Requests spent in the current window |
| `window_started` | When the current window began |

**Validation rules**

- Allowance MUST be restored by comparing `window_started` against now,
  **evaluated on access**. It MUST NOT depend on an external caller invoking a
  reset (FR-002) — that is the current bug, and a reset method with no caller
  is indistinguishable from no reset at all.
- A window that has elapsed is reset *and the current request counted against
  the new one*, so a caller returning after a long gap is served immediately
  rather than being told to wait again.
- One caller's state MUST NOT be reachable from another's path (FR-003).

**State transitions**

```
        request arrives
              │
       window elapsed? ──yes──▶ reset used=0, window_started=now
              │ no                        │
              ▼                           ▼
       used < max ? ──no──▶ Refused   used += 1 → served
              │ yes
              ▼
        used += 1 → served
```

**Reclamation (FR-004).** `client_id` is the bearer token
(`middleware.rs:93`), so the map gains an entry per distinct token and never
loses one. An entry whose window elapsed long ago carries no information — it
is indistinguishable from a caller that has never been seen — and MUST be
reclaimable on that basis.

Reclamation must happen as part of normal operation, not via a sweep something
has to remember to call. That is the same failure as `refill`, one level up.

---

## 2. Refusal — why, and when to try again

**Why it changes**: `GatewayError::RateLimited` is a unit variant
(`error.rs:22`) rendering `429` with the string `"rate_limited"`
(`error.rs:81`). The status is right; it carries no time and no cause.

| Field | Meaning |
|---|---|
| `retry_after` | How long until the caller may succeed |
| `cause` | `PerCaller` or `SystemWide` |

**Validation rules**

- `retry_after` MUST be derived from real state, not a constant. A fixed value
  that happens to be roughly right is a guess wearing a header, and SC-006
  requires a client to *sleep for it and then succeed*.
- `cause` MUST distinguish the two (FR-009): a caller over its own quota
  should back off alone; a caller refused by the system-wide ceiling is
  competing with others and should back off differently.
- Status stays `429` for both. The distinction goes in the payload and
  headers, not the status code (research R4).

**Note**: the body-size refusal is a *different* refusal with a different
status, and is not modelled here — it is a router-layer rejection that never
reaches this type.

---

## 3. Ceiling — the aggregate limit

**Why it is new**: nothing bounds total request rate today. Per-caller
allowances do not compose into a system limit: N callers each within quota can
still saturate the daemon together (FR-010).

| Field | Meaning |
|---|---|
| `max_per_window` | Requests across *all* callers per window |
| `used` / `window_started` | As Allowance, but global |

**Validation rules**

- The ceiling MUST be checked in addition to the per-caller allowance, not
  instead of it.
- A refusal by the ceiling MUST report `cause: SystemWide` (FR-009), because
  the caller's own behaviour may be blameless and its correct response differs.
- The ceiling MUST NOT be so low that ordinary multi-agent use trips it. Like
  the body limit, its value belongs to implementation where real rates can be
  observed — not to this document.

---

## What is deliberately not modelled

- **Per-route limits.** Every route shares one allowance. Differentiating
  `exec` from `list` is a plausible future refinement and explicitly not part
  of this feature.
- **Burst allowance / token-bucket smoothing.** The spec's Assumptions accept
  approximate windows; a scheme whose behaviour is obvious beats a fair one
  whose behaviour is not.
- **Persistence.** Limiter state is in-memory and lost on restart. That is
  acceptable: restart already resets everything today, and nothing depends on
  the state surviving.

---

## Relationships

```
Request ──▶ Ceiling check ──refused──▶ Refusal{ SystemWide, retry_after }
              │ passed
              ▼
          Allowance check ──refused──▶ Refusal{ PerCaller, retry_after }
              │ passed
              ▼
          scope check ──▶ handler
```

Note the order: **the limiter runs before the scope check**, so a caller
spends quota on requests it was never authorised to make. That is preserved
deliberately (research R3) — reversing it would let an unauthorised caller
probe routes for free. Anyone changing this order should say why.
