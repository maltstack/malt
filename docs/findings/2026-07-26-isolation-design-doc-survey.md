# Isolation design documents: what MALT's and vexil-v2's say, and where the code left them

**Date**: 2026-07-26
**Scope**: `docs/design/architecture.md` §"Shell and Isolation";
`~/projects/vexil-v2/docs/design-isolation.md`; the code both describe.
**Why**: before scoping the container substrate (ADR-0005), establish what has
already been decided, so the synthesis extends prior design rather than
re-deriving it.

---

## The headline: the architecture document describes a mechanism that does nothing

`docs/design/architecture.md:737-749` specifies how isolation reaches a
spawned process:

> The daemon injects an `IsolationContext` token into each MASH instance at
> creation time. When MASH calls `malt-platform` spawn traits, it passes this
> token — the platform layer reads the token and applies the appropriate
> sandbox.

Traced against the code:

| Step | State |
|---|---|
| Daemon creates the token | **Real** — `session_thread.rs:116`, `env.set_isolation_context(IsolationContext::from(isolation))` |
| MASH stores it | **Real** — `mash/src/env.rs:314`, plus `set_`/`get_`/`take_` accessors |
| Platform layer reads it and applies a sandbox | **Never happens** |

`isolation_context()` and `take_isolation_context()` have **zero callers**
anywhere in the workspace, tests included.

What actually applies isolation is a different field **six lines away in the
same struct**: `job_object: Option<Arc<JobObject>>` (`env.rs:320`), consumed
at `executor.rs:5683`. That path is real, is proven at runtime by
`capped_memory_limit_binds_where_restricted_does_not`, and is the one the
Windows tiers depend on.

So `Env` carries **two parallel isolation mechanisms**: the documented one,
which is inert, and the undocumented one, which works.

This is the ninth instance of the pattern in AGENTS.md's "Survey Before
Building", and a variant worth naming: **half-wired**. A producer exists,
storage exists, accessors exist, tests construct it — every signal a
reachability grep looks for is present except a consumer. It is more
convincing than an unwired module precisely because more of it is real.

**Consequence for ADR-0005.** The substrate must decide which mechanism is
the spine before adding backends to either. `IsolationContext` is the better
*design* — opaque token, MASH stays free of isolation logic, extends
naturally to non-Windows backends and to a privileged helper. `job_object` is
the working *code* but is Windows-specific by construction and cannot carry a
container identity. This is a real decision, not a cleanup, and it belongs in
the substrate's plan rather than being settled by whichever gets touched
first.

**Do not "fix" this by deleting `IsolationContext`.** That would remove the
only abstraction shaped correctly for what comes next.

---

## What vexil-v2's `design-isolation.md` contributes

128 lines, and better than its implementation. Five things worth carrying:

### 1. `Contained` is *defined* as image-backed

> Tier 3 | Contained | image-backed environment with the strongest isolation
> model — (line 22, restated at 47)

Images are not an enhancement to `Contained`; they are constitutive of it.
This settles a question ADR-0005 hedged: step 4 (`malt-image` + layer
materialization) is not additive work that could be deferred to make
`Contained` land sooner. A `Contained` tier without an image is a different
tier wearing the name — exactly FR-009's prohibition.

### 2. 007's thesis was written here first

> The daemon should expose what is available on the current host rather than
> pretending every tier has the same fidelity everywhere. (lines 24-26)
>
> the daemon should report unsupported tier requests clearly (line 121)

That is feature 007, stated as design intent in the predecessor project — and
vexil-v2 did not implement it either. Its capability reporting has the same
class of defect MALT's did.

**This is the strongest available argument for the ADR-0005 approach**: the
designs in these documents are sound and the implementations drifted from
them. Take the designs; re-verify every line of code.

### 3. A security expectation 007 does not cover

> tier escalation should not happen implicitly at runtime (line 117)

007's FR-017 covers containment *lost* after creation. Nothing covers
containment *gained* — a session ending up more privileged than it was
granted. Lower-probability than the fail-open, but it is the same class of
lie in the opposite direction, and no requirement currently forbids it.
**Candidate requirement for the substrate spec.**

### 4. Restore fidelity must be explicit, per tier

> The system should make restore fidelity explicit to the caller rather than
> implying every checkpoint is lossless. (lines 62-63)

The same principle as 007's FR-014 (a restored session must not inherit a
containment claim), generalised to checkpoints. vexil-v2 implements it as
`CheckpointMode::{Lossless, WarmFallback, None}` — a named-fidelity enum
directly analogous to MALT's `IsolationBasis::{Verified, Assumed, None}`.
If checkpointing is ever built here, that shape is already proven to fit.

### 5. Groups were meant to carry resource policy

The document gives groups "aggregate resource ceilings" (line 70). MALT's
`GroupPolicy` (`malt-session/src/group.rs:15`) carries `max_sessions`,
`on_empty` and `on_oom` — session-count and lifecycle policy, no resource
ceilings and no isolation policy.

Not a defect: MALT never claimed the ceilings. Recorded as a known divergence
so it is not mistaken for one later. Out of scope for the substrate.

### 6. A concrete image API surface

```text
GET  /v1/images        POST /v1/images/pull
POST /v1/images/commit GET  /v1/images/:name
```

Worth taking as the starting shape for `malt-image`'s Gateway routes, subject
to ADR-0002 (Gateway canonical) and MALT's existing route conventions —
MALT does not use a `/v1` prefix, so this is the resource model, not the
paths.

---

## What this survey did *not* establish

- **Whether vexil-v2's `oci.rs` and `image_store.rs` work.** They are wired
  to real HTTP routes and daemon state, but so was its HCS path, which
  carries a process-faulting bug. Reachability has now failed twice as
  evidence of function. Untested here.
- **Whether the remaining eleven unused `malt-platform::isolation` modules
  work.** Unchanged from the 2026-07-26 prior-art survey.
- **Whether `IsolationContext` is adequate** for a privileged-helper design,
  only that it is better shaped for it than `Arc<JobObject>`. Its actual
  fields were not audited against what a container backend would need.
- **Anything about checkpointing.** Read as design context only; no
  checkpoint code in MALT was surveyed.
