# Phase 0 Research: Gateway Hardening

**Feature**: `specs/010-gateway-hardening/`
**Date**: 2026-07-28

Every finding checked against code, with `file:line`. Two of the four problems
the request named were removed during specification because they were already
solved; the rest of this document is about what is actually broken, plus three
facts that change the shape of the fix.

---

## R1. The limiter has no clock at all

**Finding.** `crates/malt-gateway/src/rate_limit.rs` is 44 lines and imports
neither `Instant` nor `Duration`:

```rust
pub struct RateLimiter {
    max_per_window: usize,
    buckets: Mutex<HashMap<String, usize>>,
}
```

`check()` increments a counter and refuses at `max_per_window`
(`rate_limit.rs:25`). `refill()` and `refill_all()` remove entries — and have
**zero callers outside the file and its tests**.

The doc comment says "per window" and the field is `max_per_window`, but there
is no window. The name is the only thing describing time.

**Decision.** Replace rather than extend, as the spec's Assumptions state. A
counter plus an external reset cannot be made time-bounded by addition — the
missing concept is a clock, and every method's signature would change anyway.
`refill`/`refill_all` are deleted, not given a caller: a method someone must
remember to call is the shape that produced this bug (FR-002, FR-011).

---

## R2. `client_id` is the bearer token, which closes one threat and opens another

**Finding.** `middleware.rs:93`:

```rust
let ctx = AuthContext::with_client(scope, token.to_string());
```

so `ctx.client_id()` at line 95 is the token string itself.

**Two consequences, in opposite directions:**

- **Good, and it removes a spec edge case.** A caller cannot invent
  identities to evade its own limit — the token must pass
  `token_store.validate()` first (`middleware.rs:90`). The spec's "a client
  identifier is caller-controlled" edge case is therefore already answered on
  the evasion side.
- **Bad, and it is FR-004.** The map gains one entry per distinct token and
  never loses one. Nothing removes an entry when a token expires or a client
  stops calling. It is bounded only by the number of tokens ever issued.

**Decision.** Whatever replaces the counter must reclaim idle entries as part
of its normal operation, not via a separate sweep that something must
remember to call — the same reasoning as R1. Reclamation on access, or a
bounded structure, rather than a background task.

**Note for testing.** Because the identity is the token, a test needing two
distinct clients needs two valid tokens, not two arbitrary strings.

---

## R3. Quota is spent before scope is checked

**Finding.** `middleware.rs:95-103`, in order: validate token → **rate-limit
check** → scope check → run handler.

So a caller with a `Read` token hammering an `Admin` route consumes its
allowance on requests that were never going to succeed. It can exhaust its own
quota entirely on 403s.

**Decision.** Left as-is, deliberately, and recorded so it is not "fixed" by
accident. Spending quota on rejected requests is defensible — the work of
authenticating and routing was still done, and a caller probing routes it
cannot use is exactly the behaviour a limiter should discourage. Reversing the
order would let an unauthorised caller probe endlessly for free.

**This is a decision, not an oversight**, and anyone reordering these two
checks should say why.

---

## R4. The refusal already has a status; what it lacks is a time

**Finding.** `error.rs:81` maps `GatewayError::RateLimited` to
`StatusCode::TOO_MANY_REQUESTS` with code `"rate_limited"`. The status is
correct and does not need changing.

What the response carries is a status and a string. There is no
`Retry-After`, no rate-limit headers, and no way for a caller to tell a
per-caller refusal from a system-wide one — because system-wide refusals do
not exist yet (FR-010).

**Decision.** US3 adds headers and a distinguishable reason to an existing
error path rather than introducing a new one. `GatewayError::RateLimited`
gains the data it needs to render them; the status stays `429`.

---

## R5. Body limits belong in the router, not the handlers

**Finding.** No body limit exists: no `DefaultBodyLimit`, no
`content_length` check anywhere in `crates/malt-gateway/src/`.

**Decision.** Apply the limit at the router layer so it covers every route by
construction, with a larger allowance only where a route genuinely needs one.
Per-handler checks are the wrong shape for the same reason the scope map is a
latent maintainability problem (see R6): a new route added without its check
is silently unprotected.

FR-005 requires refusal *before* buffering, which a layer-level limit gives
for a declared `Content-Length`, and enforcement mid-stream for a request that
understates it (FR-007). Both halves need testing; the first is easy to get
and easy to mistake for the second.

---

## R6. What was removed, and why it is worth reading

Two of the four requested items were already done. Recorded here because
"already built" was the correct answer six times in this repo's history and
the checks that establish it are cheap:

| Claimed | Reality |
|---|---|
| `FrameWriter` casts length to `u32` unchecked (`docs/briefs/004`) | `framing.rs:203` bounds `payload_len` against `PROTOCOL_MAX_FRAME_SIZE` and returns `FrameTooLarge` before the cast at line 209. Both failure modes prevented. Brief marked **RESOLVED** |
| Per-endpoint scope not enforced route by route | `middleware.rs:30-32` maps `(Method, path)` to a scope, defaulting `_ => AuthScope::Admin`. An unmapped route demands the **highest** scope |

The scope map is still two sources of truth with the router, and a new route
that forgets an entry becomes Admin-only — an availability bug, not a
security one. Out of scope here; belongs in the backlog.

**Consequence.** Removing brief 004 removed the entire VNP half of this
feature, which settles the design question the request posed: the two
boundaries do **not** need matching treatment, because VNP's is already
symmetric. Planning does not need to decide this.

---

## R7. This feature's specific failure mode

*(Required by the plan template's standing rules.)*

**Asserting the status code and calling it verified.**

Every requirement here has an observable that is *easier* to check than the
one that matters, and the easy one passes while the defect is fully present:

| Easy assertion | What it misses |
|---|---|
| "429 after N requests" | The current limiter does this correctly. It is the *recovery* that is broken, and a test that never waits will never see it |
| "413 for a large body" | Says nothing about whether the body was buffered first, which is the actual harm (SC-004) |
| "the map has one entry per client" | Says nothing about whether entries are ever released (SC-008) |
| "a `Retry-After` header is present" | Says nothing about whether its value is usable — a client must be able to sleep for it and succeed (SC-006) |

So: **no task here is complete on the strength of a status-code assertion.**
Tests must advance time, measure memory, or drive a client that actually
recovers. Where a test cannot observe the real property, it says so rather
than asserting the proxy.

---

## Gates that apply

- `cargo build --workspace`
- `cargo test --workspace` — **note `MASH` must be set**, or the Smoosh test
  fails and looks like a regression:
  `$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings` — the `-D warnings`
  matters; a warn-level lint once blocked CI's test step for ~110 commits
- Linux via `bash scripts/wsl-mirror.sh`, not `/mnt/c`

**Smoosh does not apply** — neither `mash` nor `malt-tools` is touched. Stated
so its absence is not read as an oversight.

**macOS is not a target** (ADR-0006) and is absent from CI.

---

## Open questions carried into Phase 1

- **The actual limit values.** FR-006 requires the body bound not to break
  ordinary use, and SC-005 requires proving it with a real payload. The number
  should come from measuring the largest legitimate `exec`/`send` bodies, not
  from a guess.
- **How to observe memory for SC-004 and SC-008.** The requirement is settled;
  the measurement technique is not. If it cannot be observed reliably in a
  test, say so and state what is asserted instead — do not silently downgrade
  to a status-code assertion, which is R7 exactly.
