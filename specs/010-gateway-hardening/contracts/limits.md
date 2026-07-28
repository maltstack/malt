# Contract: limits, refusals, and how a caller reads them

**Feature**: `specs/010-gateway-hardening/`

Two surfaces change: what a refusal looks like on the wire, and what
configuration an operator has. They are specified together because a limit an
operator cannot see or tune is a limit that gets worked around.

---

## 1. Refusal on the wire

### Rate refusal — status unchanged, payload enriched

`429 Too Many Requests` stays. `error.rs:81` already maps it correctly; only
the accompanying data is new.

```json
{ "ok": false, "error": {
    "code": "rate_limited",
    "message": "request allowance exhausted; retry after 12s",
    "cause": "per_caller",
    "retry_after_secs": 12 } }
```

Headers:

```
Retry-After: 12
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 12
```

`cause` is `per_caller` or `system_wide`. **Both are 429.** A caller over its
own quota backs off alone; a caller refused by the ceiling is competing with
others and may need to back off harder or longer. Distinguishing them by
status code was considered and rejected — 503 for the ceiling would conflate
"too busy" with "unavailable", which `ExecutionQueueFull` already means
(`error.rs:83`).

**`Retry-After` must be honest.** A caller that sleeps for it and retries MUST
succeed, absent new contention (SC-006). A padded constant is not acceptable:
it makes the header useless for the well-behaved caller it exists to serve.

### Size refusal — a different refusal entirely

`413 Payload Too Large`, produced at the router layer before the body is read.
It does not pass through `GatewayError::RateLimited` and carries no
`Retry-After` — retrying an oversized body unchanged will fail identically.

```json
{ "ok": false, "error": {
    "code": "payload_too_large",
    "message": "request body exceeds the 1 MiB limit" } }
```

The message MUST state the limit. A caller told only "too large" has to
bisect to find the boundary.

---

## 2. Successful responses

Rate-limit headers on **every** response, not only refusals:

```
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 87
X-RateLimit-Reset: 43
```

This is what lets a caller pace itself instead of discovering the limit by
hitting it. A limiter that only speaks when refusing teaches clients to probe.

---

## 3. Configuration

| Setting | Default | Governs |
|---|---|---|
| per-caller allowance | *(implementation)* | FR-001, FR-003 |
| window length | *(implementation)* | FR-001 |
| system-wide ceiling | *(implementation)* | FR-010 |
| request body limit | *(implementation)* | FR-005 |

**The defaults are deliberately left to implementation**, and this is not an
omission. FR-006 requires the body bound not to break ordinary use and SC-005
requires proving it with a real payload — so the number must come from
measuring actual `exec` and `send` bodies, not from a spec guessing. The same
applies to rates.

Two constraints on whatever is chosen:

- **A daemon started with no configuration MUST still be protected.** Defaults
  are safe values, not "unlimited".
- **Each MUST be observable at runtime**, so an operator diagnosing a refusal
  can see the limit that produced it rather than reading source.

---

## 4. Compatibility

- **Behavioural change, and the point of the feature**: callers currently
  refused permanently after N requests will start being served again. Nothing
  that succeeds today begins failing, with one exception — requests above the
  new body limit, which were previously buffered without bound.
- **No wire-shape break.** `429` stays `429`; the payload gains fields and the
  response gains headers. A client ignoring both behaves exactly as now.
- **`refill` and `refill_all` are removed** from `RateLimiter`. They have no
  production callers (verified 2026-07-28), so nothing outside the crate's own
  tests can notice.
