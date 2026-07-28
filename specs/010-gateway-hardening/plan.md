# Implementation Plan: Gateway Hardening

**Branch**: `010-gateway-hardening` | **Date**: 2026-07-28 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/010-gateway-hardening/spec.md`

## Summary

Give the Gateway limits an untrusted caller cannot escape and a well-behaved
one can live with: a rate limiter that actually recovers, a bound on request
bodies enforced before buffering, and refusals that tell a caller when to try
again.

Phase 0 confirmed the shape is smaller and sharper than the request assumed.
Two of the four items were already solved and were removed at specification
time (research R6), which also settled the "do both boundaries need the same
treatment" question — VNP's is already symmetric, so this is HTTP-only.

The work concentrates in one 44-line file. `rate_limit.rs` has no clock at
all — it imports neither `Instant` nor `Duration` — so US1 is a replacement,
not a repair.

## Technical Context

**Language/Version**: Rust, edition 2021 (workspace `rust-version`)

**Primary Dependencies**: `axum` 0.8 (already present; `DefaultBodyLimit`
covers FR-005 at the router layer), `std::time` for the window. No new
external dependency is expected — if one is proposed, it needs justifying
against a `HashMap` and an `Instant`.

**Storage**: None. Limiter state is in-memory and deliberately reclaimable
(FR-004); nothing here persists.

**Testing**: `cargo test`; existing `crates/malt-gateway/tests/rate_limit.rs`
and `routes.rs` are the homes for most of this. Route tests use a mock backend
via `tower::ServiceExt::oneshot`.

**Target Platform**: Windows (primary, blocking gates) and Linux (advisory,
reproducible via `scripts/wsl-mirror.sh`). macOS is not a target — ADR-0006.

**Project Type**: Multi-crate Rust workspace. This feature touches
`malt-gateway` (L2) only, plus its tests.

**Performance Goals**: Not a performance feature, but the limiter is on every
request. Whatever replaces the `Mutex<HashMap>` must not make the hot path
meaningfully worse — one lock and one map lookup is the bar to match.

**Constraints**: The limiter is **live in `build_router`** today, so every
change ships to every route at once. There is no partial rollout and no
feature flag; correctness here is not optional.

**Scale/Scope**: One file rewritten (`rate_limit.rs`, 44 lines), one
middleware call site adjusted, one router-layer limit added, one error variant
enriched.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Note |
|---|---|---|
| **I. VT codes confined** | ✅ N/A | No VT handling |
| **II. OS calls confined** | ✅ N/A | `std::time` is not an OS abstraction in the sense this invariant governs; no `nix`/`windows-sys`/`libc` |
| **III. Dependency-free foundations** | ✅ | `malt-protocol` and `malt-plugin-sdk` untouched — the VNP half was removed at spec time |
| **IV. Safety is explicit** | ✅ **Watch item** | No `unsafe` expected. `rate_limit.rs` already uses the poison-tolerant `.lock().unwrap_or_else(\|e\| e.into_inner())` that AGENTS.md requires — **keep it**; reverting to `.unwrap()` reintroduces a documented cascade |
| **V. VNP only** | ✅ N/A | HTTP surface only |
| **VI. Smoosh gate** | ✅ **Does not apply** | Neither `mash` nor `malt-tools` is touched |
| **VII. Layer violations** | ✅ | `malt-gateway` is L2 and depends downward only |
| **VIII. Vendor, never depend** | ✅ | No sibling-project code involved |
| **IX. No silent scope-jumps** | ✅ | The scope-map maintainability concern (research R6) is named out of scope rather than absorbed |
| **X. Commit at checkpoints** | ✅ | Three user stories, three natural commits |

**Verdict: PASS.** No violations to justify; one watch item (IV).

## Project Structure

### Documentation (this feature)

```text
specs/010-gateway-hardening/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── limits.md        # Phase 1 output — refusal shape and headers
├── checklists/
│   └── requirements.md
└── tasks.md             # Created by /speckit-tasks
```

### Source Code (repository root)

```text
crates/malt-gateway/
├── src/
│   ├── rate_limit.rs    # REWRITTEN — the window, and reclamation of idle
│   │                    #   entries. refill/refill_all are DELETED, not
│   │                    #   given callers (research R1)
│   ├── middleware.rs    # MODIFIED — carry the refusal's retry data out of
│   │                    #   the limiter; the check/scope ORDER stays as-is
│   │                    #   and is a recorded decision (research R3)
│   ├── error.rs         # MODIFIED — RateLimited carries retry-after and
│   │                    #   whether the cause was per-caller or system-wide.
│   │                    #   Status stays 429 (research R4)
│   └── server.rs        # MODIFIED — router-layer body limit (research R5)
└── tests/
    ├── rate_limit.rs    # EXTENDED — recovery, isolation, reclamation
    └── routes.rs        # EXTENDED — body limits, refusal headers
```

**Structure Decision**: Everything stays inside `malt-gateway`. The body limit
goes at the **router layer**, not in handlers, so it covers every route by
construction — a per-handler check is the shape that lets a newly added route
be silently unprotected, which is the same weakness the scope map already has
(research R5, R6).

## Complexity Tracking

*No Constitution violations require justification.*

One choice worth flagging as deliberate rather than complex:

| Choice | Why | Alternative rejected because |
|---|---|---|
| Delete `refill`/`refill_all` rather than wiring them up | FR-002, FR-011 | Giving them a caller keeps a mechanism that only works if something remembers to invoke it — exactly the state that produced this bug, and another instance of AGENTS.md failure class 1 |

## Phase 1 outputs

- [data-model.md](./data-model.md) — allowance, refusal, ceiling
- [contracts/limits.md](./contracts/limits.md) — refusal shape, headers,
  configuration surface
- [quickstart.md](./quickstart.md) — one runnable scenario per success
  criterion

## Post-Design Constitution Re-check

| Principle | Status after design |
|---|---|
| **IV. Safety explicit** | ✅ Poison-tolerant locking retained; no new `unsafe`; no `unwrap` outside tests |
| **VII. Layering** | ✅ Confined to `malt-gateway` |
| **IX. No silent scope-jumps** | ✅ Scope-map concern remains out of scope, recorded in research R6 and the spec |

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
