# Brief 003 — The rate limiter has no window

**Severity**: High · **Verified**: 2026-07-26 · **Source**: audit A-05

## What is wrong

`crates/malt-gateway/src/rate_limit.rs:7`:

```rust
/// Simple per-client rate limiter with a fixed token count per window.
pub struct RateLimiter {
    max_per_window: usize,
    buckets: Mutex<HashMap<String, usize>>,
}
```

There is no window. The file contains no `Instant`, no `elapsed`, no refill,
no reset — the counter only ever increments. A client that makes
`max_per_window` requests is refused **for the lifetime of the daemon**.

The doc comment says "per window", which is how this survived: the type reads
correctly at a glance, and the name of the field asserts the behaviour the
code does not implement.

## Why it matters

It is worse than having no rate limiting. No limiter means abuse is possible;
this means **a legitimate client is permanently banned** at an arbitrary
point, with recovery only by restarting the daemon — which also destroys
every running session.

It became reachable when Gateway auth was wired into `build_router`
(2026-07-25). Before that the limiter was never consulted, which is why
nothing had hit it.

## What done looks like

- A real window: either a fixed window that resets, or a token bucket that
  refills — the choice matters less than that time enters the calculation.
- A client that stops for the window length can make requests again, with
  **no daemon restart**.
- The limit is per-credential, and one client exhausting it does not affect
  another.
- A test that advances time (or uses a short window) and shows recovery.
  Asserting only that the Nth request is refused passes against the current
  broken implementation.
- Entries are evicted, or the map grows without bound for every credential
  ever seen.

## Gotchas

- **Fix the doc comment with the code.** "per window" on a type with no
  window is what made this invisible; leaving it would preserve the trap.
- Check whether the limit is applied per route or globally before changing
  it — a limit tuned for cheap reads may be wrong for `exec`.
- The `Mutex<HashMap>` is on the request path; keep the critical section
  short. A poisoned lock here would take the whole Gateway down — see
  [brief 001](001-hard-invariant-violations.md) on poison handling.
