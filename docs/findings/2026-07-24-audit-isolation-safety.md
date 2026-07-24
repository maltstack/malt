# Audit: Isolation Fail-Open Safety Gaps

Date: 2026-07-24
Context: The project owner raised a specific, direct concern: session
isolation setup is currently best-effort everywhere, and a session created
with `Restricted`/`Capped`/`Contained` can silently run exactly as `Bare` if
the underlying OS containment mechanism fails to set up. This matters more
than it would in a general-purpose terminal because MALT is explicitly
positioned to be driven by autonomous agents — "ambiguous containment"
under agent-issued commands is a materially worse failure mode than under a
human at a keyboard. This audit catalogs every enforcement point end to end,
confirms the known lead, and finds several more gaps of the same shape that
weren't in the original brief. Read-only investigation; no source or other
docs were modified.

Method: read every call site in the isolation chain from the VNP/HTTP
request boundary down to the raw Win32 calls, cross-referencing AGENTS.md's
and `docs/design/architecture.md`'s claims against the actual code (not
assumed from docs). Followed the same evidence standard as
`docs/findings/2026-07-24-live-daemon-session.md`: file:line citations, not
paraphrase.

## Summary — the picture is worse than the brief assumed

The originating concern was about Windows Job Object creation failing and
the session continuing anyway. That's real (item 1 below). But tracing the
full chain surfaced three additional facts, each independently sufficient to
make `required` semantics necessary, not just nice-to-have:

1. **On Linux and macOS, no isolation mechanism is invoked at spawn time at
   all**, for any tier, success or failure. The only per-platform code that
   actually *runs* at spawn time today is the Windows Job Object path.
   Linux's namespace/cgroup/overlayfs/seccomp/network modules and macOS's
   sandbox/rlimit modules are called **only** from the capability-probing
   function (`tier_available`), never from anything that spawns a process.
   This isn't "unverified" (AGENTS.md's phrasing) — it's not wired to the
   spawn path at all, on either platform.
2. **Even where the mechanism does run (Windows Job Objects), it is
   tier-blind.** `apply_session_isolation` creates the *same* Job Object
   with *zero* memory/CPU limits for Restricted, Capped, and Contained
   alike. So even on a fully successful path, a `Capped` or `Contained`
   session gets exactly what a `Restricted` session gets: group-kill
   capability and nothing else. No resource caps, no privilege
   restriction, no HCS container — regardless of tier.
3. **The client-side safety check that looks like it should catch this
   (`malt-bin`'s `validate_created_session`) cannot ever catch it**, because
   the value it compares against is computed before isolation setup runs
   and is never updated by the outcome (see item 7).

## The catalog

### 1. Windows Job Objects — fail-open, confirmed, tier-blind

`crates/malt-daemon/src/executor/session_thread.rs:30-52`
(`apply_session_isolation`):

```rust
fn apply_session_isolation(env: &mut Env, session_id: SessionId, isolation: IsolationTier) {
    env.set_isolation_context(malt_platform::isolation::IsolationContext::from(isolation));

    #[cfg(windows)]
    {
        if isolation == IsolationTier::Bare {
            return;
        }
        let job_name = format!("malt-session-{}", session_id.0);
        match malt_platform::isolation::job_objects::create_job_object(&job_name, 0, 0) {
            Ok(job) => env.set_job_object(std::sync::Arc::new(job)),
            Err(error) => warn!(..., "failed to create job object for session isolation; \
                session will run without process containment"),
        }
    }
    #[cfg(not(windows))]
    { let _ = session_id; }
}
```

**Fail-open, confirmed.** `Err` from `create_job_object` becomes a `warn!`
log line and nothing else — the session thread proceeds identically to the
success path. This function returns `()`, not `Result`, so there is
structurally no way for the caller to know isolation setup failed. Called
from `SessionExecutor::spawn` (line 198) and `spawn_with_cwd` (line 233),
both inside the spawned session thread — i.e. after the *parent* thread has
already committed to returning `Ok` from `SessionExecutor::spawn` (see item
7 for why this ordering matters for the fix).

**Tier-blind, not previously flagged.** `create_job_object(&job_name, 0, 0)`
— the two trailing arguments are `memory_limit_mb` and `cpu_rate`
(`crates/malt-platform/src/isolation/job_objects.rs:128-132`, doc comment:
`0 = no limit` for both). This call is made identically for `Restricted`,
`Capped`, and `Contained` — the only branch in the whole function is
Bare-vs-not-Bare (line 35). So even when Job Object creation *succeeds*,
`Capped` (which architecture.md's tier table promises "Extended Job Object
limits") gets no memory or CPU limit, and `Contained` (promised "HCS
containers... or AppContainer") gets nothing beyond the same bare Job
Object a `Restricted` session gets. `assign_child_to_session_job` in
`crates/mash/src/executor.rs:5566-5581` is the only other Windows-side
enforcement point (assigns each spawned child PID to the job) and is itself
fail-open by design (comment at line 5563: "a failure here doesn't fail the
command — the child still runs, just outside containment").

### 2. Restricted tokens — real and fail-closed at the function level, but unreachable from any spawn path

`crates/malt-platform/src/isolation/tokens.rs` — `create_restricted_token`
(line 121), `create_sandbox_token` (line 231), `get_current_token` (line
239) all correctly return `Err(IsolationError::TokenError(...))` on Win32
failure (e.g. lines 138-142, 168-172, 246-249). AGENTS.md's "real, tested"
claim is accurate at the unit level — these are genuine Win32 calls with
real tests exercising them (`create_restricted_token_actually_succeeds`,
etc., lines 311-358), not stubs.

**But nothing calls them.** `grep -rn "create_restricted_token\|create_sandbox_token"` across the
whole workspace outside `tokens.rs` itself returns only
`crates/malt-elevate/src/dispatch.rs` — and that reference is the
`CreateRestrictedToken` *enum variant name*, matched by a stub arm (line
52) that never calls into `tokens.rs` at all (see item 4). No code in
`malt-daemon`, `mash`, or anywhere in the live session-creation path
constructs a `RestrictedToken` or attaches one to a spawned process. This
function is **fail-closed but dead code** — a correct implementation
sitting unreferenced. Restricted-tier sessions on Windows get a Job Object
today (item 1) and nothing from `tokens.rs`, even though architecture.md's
tier table (line 1526) lists "Job Objects, restricted tokens" together as
what `Restricted` provides.

### 3. HCS bindings — fail-closed at the function level, called only by the capability prober, never by session creation

`crates/malt-platform/src/isolation/hcs.rs`. Every public entry point
(`create_compute_system` line 148, `open_compute_system` line 167,
`create_process` line 208, etc.) validates preconditions and propagates
real errors (`Err(IsolationError::HcsError(...))`) — including
`ensure_hcs_runtime()` (line 104), which explicitly fails if the `hcs`
Cargo feature isn't compiled in, if `computecore.dll` is missing, or if
required Win32 symbols don't resolve. If the real `HcsCreateComputeSystem`
call fails at runtime (native path, line 407-459), the HRESULT is captured
and returned as `Err` (lines 432-441) — this genuinely reaches whatever
calls `create_compute_system`.

**The problem: nothing in the session-creation path calls it.**
`grep -rln "isolation::hcs" crates/` (excluding `hcs.rs` itself) returns
exactly one file: `crates/malt-platform/src/isolation/probe.rs`, i.e. HCS is
invoked only during `tier_available()`'s capability check (see item 6), not
during actual session isolation setup. `apply_session_isolation` (item 1)
never calls `hcs::create_compute_system` for `Contained`-tier sessions — it
runs the exact same tier-blind Job Object path as `Restricted`/`Capped`. So
today, a `Contained` session on Windows gets **no HCS container at all**,
regardless of whether HCS is actually available on the host. This
contradicts AGENTS.md's Phase H line ("HCS bindings (real Win32 calls,
feature-gated, only fake-mode tested)") only in emphasis — the bindings
genuinely work in fake mode and are real Win32 calls, but the missing piece
isn't "unverified in real mode," it's "never invoked by anything a user's
session creation actually reaches."

### 4. `malt-elevate` — stub-lies-about-success, and currently unreachable from any code path (latent, not live)

`crates/malt-elevate/src/dispatch.rs:44-59` (`dispatch_request`), stub arms
with exact line numbers:

| Operation | Line | 
|---|---|
| `CreateNamespace` | 46 |
| `MountOverlay` | 47 |
| `SetCgroup` | 48 |
| `SetupNetns` | 49 |
| `ApplySeccomp` | 50 |
| `CreateRestrictedToken` | 52 |
| `ManageHcsContainer` | 53 |
| `ApplySeatbelt` | 54 |
| `BindPort` | 55 |

All nine route through `stub_success()` (lines 62-65):

```rust
fn stub_success(op: &str) -> Result<Vec<u8>, String> {
    tracing::info!(operation = op, "stub: operation not yet implemented, returning success");
    Ok(Vec::new())
}
```

This is the worst category in the catalog: not fail-open (which at least
implies an attempt was made) but **actively reports success while doing
nothing**. Only `CreateSymlink` (line 51, `dispatch_create_symlink`) does
real work.

**Reachability: confirmed unreachable from any live path today.**
`grep -rn "malt-elevate" crates/*/Cargo.toml` shows no crate depends on
`malt-elevate` as a library — it's a standalone binary with its own
`Cargo.toml`. `grep -rln "malt_elevate\|malt-elevate" crates/malt-daemon
crates/mash crates/malt-platform crates/malt-bin` finds a single hit, a doc
comment in `hcs.rs:270` referencing it in prose, not code. Neither
`malt-daemon` nor `malt-bin` spawns the `malt-elevate` binary as a
subprocess anywhere. **This means the stub-lies-about-success behavior is
latent risk, not live risk** — no user action today routes through it. The
severity is: it will become live the moment anything wires
elevation-requiring isolation operations through it (which is exactly what
real Linux namespace/cgroup/seccomp support and real restricted-token
issuance in unprivileged mode would require, per architecture.md §14
"Privilege Separation"). It should be fixed (or at minimum gated so it
cannot silently ship as "done") before that wiring lands, not treated as
urgent today.

### 5. PTY/compat supervisor spawn path — no isolation wiring, and currently unreachable (latent, not live)

`crates/malt-daemon/src/supervisor/mod.rs:29-70` (`ProcessSupervisor::spawn`)
never references isolation, Job Objects, or the `isolation` field at all.
Notably, `SpawnRequest` (`crates/malt-daemon/src/supervisor/process.rs:12-20`)
**does** have an `isolation: IsolationTier` field —

```rust
pub struct SpawnRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub pane_id: PaneId,
    pub isolation: IsolationTier,
    pub cols: u16,
    pub rows: u16,
}
```

— but `grep -n "req.isolation\|\.isolation" crates/malt-daemon/src/supervisor/*.rs`
returns zero hits inside `spawn()`. The field is populated by test fixtures
(`crates/malt-daemon/tests/supervisor.rs`, `tests/process.rs`) but never
read by production code. This is a stronger version of BACKLOG.md's P2
description ("no isolation wiring") — the type already carries the tier,
someone clearly intended to wire it, and it's simply never consulted.

**Reachability: currently unreachable from any live user action.**
`grep -n "supervisor" crates/malt-daemon/src/executor/coordinator.rs` shows
`Coordinator` holds a `supervisor: ProcessSupervisor` field (constructed at
line 105) but **never calls `.spawn()` on it anywhere**. Cross-referencing
BACKLOG.md's P1 item confirms why: Compat panes can only be created via the
session-restore path today, and that path is a confirmed stub
(`coordinator.rs:547-551`, `DaemonError::RestoreFailed(id, "compat pane
restore not yet implemented")` — `spawn_compat` itself was never written).
So today, no user action reaches `ProcessSupervisor::spawn` at all — this
gap is real but latent, exactly parallel to item 4. It becomes live risk
the moment Phase B2's compat-pane restore or any future PTY-pane feature
gets implemented, so the isolation wiring should land no later than that
work, not be treated as "already exposed today."

### 6. `tier_available()` — informational only, never consulted by session creation

`crates/malt-platform/src/isolation/mod.rs:53` (`tier_available`), doc
comment at lines 50-52: *"This function performs lightweight runtime probes
— it does not actually create namespaces, job objects, or sandboxes, only
checks prerequisites."* That's accurate and expected — probing is supposed
to be side-effect-free. The problem is the consumer side:
`grep -rn "tier_available" crates/` shows the function is called **only
from its own test module** (`mod.rs:173,198,199,201,208,209,210`). No call
site in `malt-daemon`'s `coordinator.rs`, `session_thread.rs`,
`gateway_backend.rs`, or `malt-gateway`'s routes ever calls it. This
directly contradicts architecture.md's stated intent (line 1567): *"Clients
check `tier_available(tier)` — if a requested tier isn't supported, the
session creation fails with a clear error explaining what's available, not
a silent fallback to Bare."* Today, a session creation request for a tier
the host cannot support at all (e.g. `Contained` on a Windows box without
Hyper-V) is never rejected up front — it falls straight through to
`apply_session_isolation`, which (item 1) will either coincidentally
succeed at creating a bare Job Object (since Job Object creation doesn't
actually require the capabilities `Contained` implies) or fail and warn.
Either way, the user never gets the "explicit error explaining what's
available" the design promised.

### 7. Session-creation return type — no plumbing exists today; here's exactly what's missing

Traced the full chain from HTTP request to session thread:

```
POST /sessions (isolation: "restricted")
  → malt-gateway routes/sessions.rs:21-23 (create_session route handler)
    → gateway_backend.rs:49-67 (DaemonBackend::create_session)
        - parse_isolation() (line 24-31): unrecognized strings silently
          become Bare (`_ => IsolationTier::Bare`), not an error. A typo'd
          isolation value ("resticted") is a fourth, independent silent
          fail-open, distinct from the Job Object question.
        - coord.create_session(name, tier, None) (line 56-58): any
          DaemonError → GatewayError::Internal(e.to_string()), which
          malt-gateway's error.rs:52 already maps to HTTP 500. This part
          of the chain already works — an error here WOULD reach the
          client today.
      → coordinator.rs:115-175 (Coordinator::create_session)
          - SessionExecutor::spawn(...)? (line 155) — already propagates
            via `?`. If spawn() returned an isolation-related Err, it
            would reach the gateway with zero changes needed here. Also:
            the session is only inserted into `self.sessions` (line 159)
            AFTER spawn() returns Ok, so an Err here leaves no partial
            state to clean up — safe to hook into as-is.
        → session_thread.rs:187-216 (SessionExecutor::spawn)
            - Returns Result<(tx, handle), DaemonError>, but the ONLY
              possible Err today is DaemonError::Io from
              thread::Builder::spawn() failing to create an OS thread
              (line 214) — completely unrelated to isolation.
            - apply_session_isolation() (line 198) runs INSIDE the
              spawned thread's closure, AFTER the parent thread's
              thread::Builder::spawn() call has already returned Ok. The
              parent has no way to observe what happens inside the
              closure — by the time apply_session_isolation's warn! fires,
              SessionExecutor::spawn has already returned Ok((tx, handle))
              to the coordinator, which has already returned Ok(session_id)
              to the gateway, which has already returned HTTP 200.
```

**What would actually be needed for `required` semantics, concretely:**

1. `apply_session_isolation` (`session_thread.rs:30`) needs to return
   `Result<(), IsolationError>` instead of `()`.
2. `SessionExecutor::spawn`/`spawn_with_cwd` (`session_thread.rs:187`, `221`)
   need a synchronization point between the spawned thread and the caller
   — e.g. a rendezvous channel (`mpsc::sync_channel(0)`) that the closure
   sends an `IsolationSetupResult` on *before* proceeding to
   `executor.run(rx)`, and that the parent blocks on before returning. This
   is a structural change, not a signature tweak: today the parent thread
   returns unconditionally as soon as the OS thread is created, without
   waiting for anything the closure does.
3. `DaemonError` (`crates/malt-daemon/src/error.rs:5-46`, already
   `#[non_exhaustive]`) needs a new variant, e.g.
   `IsolationRequired(SessionId, IsolationTier, String)`, following the
   existing pattern of `RestoreFailed(SessionId, String)` (line 42).
4. `Coordinator::create_session` (`coordinator.rs:115`) needs **no
   change** — its existing `?` on `SessionExecutor::spawn(...)` (line 155)
   already propagates any new `DaemonError` variant.
5. `gateway_backend.rs::create_session` (line 49-67) needs **no change**
   to propagate the failure (its `.map_err(...)` at line 58 already
   forwards any `DaemonError`), but should probably map an isolation
   failure to a more specific `GatewayError` variant than `Internal` (see
   Proposal below) so the HTTP status is 4xx ("you asked for something
   unsatisfiable") rather than 500 ("the server is broken").
6. `parse_isolation` (`gateway_backend.rs:24`) needs its silent
   `_ => IsolationTier::Bare` fallback (line 29) replaced with an explicit
   `GatewayError::BadRequest` for unrecognized isolation strings — today an
   API caller with a typo in the isolation field gets a session that's
   silently Bare with no indication anything was wrong.
7. **The one place that looks like a safety net today,
   `malt-bin`'s `validate_created_session`
   (`crates/malt-bin/src/main.rs:131-145`), cannot detect any of the above
   failures.** It compares `session.isolation` (the string
   `gateway_backend.rs:64` echoes back, computed from the *locally parsed
   request tier* at line 54, before `apply_session_isolation` ever runs)
   against the tier the CLI requested. Since the echoed value is never
   updated by what actually happened on the session thread, this check can
   only ever catch a coordinator-level tier substitution (e.g. a future bug
   where `create_session` itself picks a different tier than requested) —
   it is structurally blind to a Job Object failure, because nothing
   currently threads "did isolation setup actually succeed" back into the
   value being compared.

## Proposal: where a `required`/`preferred`/`disabled` policy field should live

**Recommendation: a new field alongside `IsolationTier`, not a change to
`IsolationTier` itself.** `IsolationTier` is documented as "Fixed set —
platform capabilities are finite" (`schemas/common.vexil:28`) — it encodes
*what* containment is requested. Whether a failure to establish it should
be fatal is an orthogonal axis (*how strictly* to enforce it), and
conflating them would double the enum's cardinality for no reason (every
tier already has a `Bare` fallback path; what's missing is control over
whether that fallback is allowed to happen silently).

Concretely:

- **Schema**: add a new enum to `schemas/common.vexil`, e.g.

  ```
  @doc("How strictly a requested isolation tier must be honored.")
  enum IsolationPolicy {
      Required  @0   # failure to establish the tier must prevent the session from starting
      Preferred @1   # best-effort: fall back to a weaker tier (today's behavior) on failure
      Disabled  @2   # explicitly opt out of isolation regardless of tier (documents intent,
                     # distinct from omitting the field, which currently defaults to best-effort)
  }
  ```

  Add `policy @2 : IsolationPolicy` to `CreateSession`
  (`schemas/session.vexil:10-14`, next available field number after
  `group @2` — would need renumbering to `policy @3` or appending after
  `group`) and to `PersistedSession`'s isolation field
  (`schemas/persist/session.vexil:18`) so a restored session remembers
  what was actually asked for. Default (omitted field) should map to
  `Preferred` — this preserves 100% of today's observed behavior for any
  caller that doesn't opt in, which matters given `malt new`'s existing
  isolation-tier default-to-Bare compatibility test
  (`crates/malt-bin/src/cli.rs` — "preserves the omitted-field/Bare
  default").

- **Call sites needing a policy check** (new logic, not just plumbing):
  - `session_thread.rs::apply_session_isolation` (line 30) — the actual
    decision point. Needs the signature change described in item 7 above,
    plus: on `Err` from `create_job_object`, check the policy; only `warn!`
    and continue if `Preferred`, return `Err` if `Required`.
  - Eventually, the equivalent Linux/macOS enforcement code, once item 1's
    finding (no real enforcement exists yet on those platforms) is
    separately fixed — this audit does not recommend adding `required`
    checks to functions that don't yet call any real containment
    mechanism; that would let `required` succeed by doing nothing, which is
    worse than today's honest fail-open. Sequencing matters: real
    Linux/macOS enforcement should land before or alongside `required`
    semantics for those platforms, not after.

- **Call sites needing new error-propagation plumbing** (structural, per
  item 7):
  - `session_thread.rs::SessionExecutor::spawn` / `spawn_with_cwd`
    (lines 187, 221) — rendezvous channel + `Result` change.
  - `crates/malt-daemon/src/error.rs` — new `DaemonError` variant.
  - `gateway_backend.rs::create_session` (line 49) — map the new variant to
    a dedicated `GatewayError` (e.g. `IsolationUnsatisfiable`) distinct from
    `Internal`, mapped to `422 Unprocessable Entity` or `400 Bad Request` in
    `malt-gateway/src/error.rs` (currently `Internal` → 500 at line 52) —
    this is a "the request was well-formed but the guarantee can't be met,"
    not a server bug.
  - `gateway_backend.rs::parse_isolation` (line 24) — needs to return
    `Result<IsolationTier, GatewayError>` instead of silently defaulting
    unrecognized strings to `Bare` (line 29), independent of the
    `required`/`preferred` question but adjacent and cheap to fix at the
    same time.
  - `crates/malt-gateway/src/types.rs::CreateSessionRequest` (line 37-40) —
    add `pub policy: Option<String>` (or a typed enum if malt-gateway
    generates from the schema) alongside `isolation`.
  - `crates/malt-bin/src/cli.rs`/`client.rs` — add a `--isolation-policy`
    (or fold into `--isolation` as e.g. `restricted:required`) CLI surface,
    and fix `validate_created_session` (`main.rs:131-145`) to be
    meaningful again — once the gateway can actually report a real failure,
    this function's existing structure (compare requested vs. reported)
    becomes useful for the first time instead of decorative.

## Severity ranking — reachable today vs. latent

Ordered by how directly a real user-facing action reaches the gap right
now, most urgent first:

1. **Windows Job Object fail-open + tier-blindness (items 1)** — LIVE
   TODAY. `malt new --isolation restricted|capped|contained` (shipped
   2026-07-24 per BACKLOG.md) reaches this exact code path on every
   Windows session creation. This is the one the project owner named, and
   it's confirmed live, confirmed fail-open, and confirmed to additionally
   under-deliver even on success (no tier differentiation).
2. **`tier_available()` never consulted (item 6)** — LIVE TODAY. Every
   session creation on every platform skips this check entirely, so a
   request for a tier the host genuinely cannot support at all gets no
   up-front rejection.
3. **Silent Bare fallback on unparseable isolation strings (item 7,
   `parse_isolation`)** — LIVE TODAY, lowest-effort fix in this list. Any
   API caller (including a coding agent constructing the JSON body itself)
   that mis-spells or mis-cases the isolation field gets a silently-Bare
   session with a 200 response.
4. **No real Linux/macOS enforcement at all (item 1's cross-platform
   finding)** — LIVE TODAY on those platforms, in the sense that
   `Restricted`/`Capped`/`Contained` requests succeed and report success
   while providing literally zero containment, on any non-Windows host.
   Ranked below the Windows items only because this repo's active
   development and testing has been Windows-first (per AGENTS.md's
   Windows-specific PATH/build notes); the gap is arguably *more* severe in
   effect (total absence vs. partial/tier-blind), just less immediately
   discoverable from this environment.
5. **Restricted tokens and HCS being fail-closed-but-dead (items 2, 3)** —
   LIVE TODAY in the sense that Windows sessions run without them right
   now, but LOWER urgency than items 1/2/3 above because the functions
   themselves are already correct — the fix is wiring, not correctness
   work, and wiring them in is naturally downstream of fixing item 1's
   tier-blindness (you'd wire tokens/HCS in as part of making
   `apply_session_isolation` actually branch on tier).
6. **`malt-elevate` stub-lies-about-success (item 4)** — LATENT. Confirmed
   unreachable from any code path today; becomes live only when something
   starts routing privileged operations through it.
7. **PTY/compat supervisor spawn path unwired (item 5)** — LATENT.
   Confirmed unreachable today because Compat-pane creation itself is a
   stub (BACKLOG.md P1); becomes live only when compat-pane restore or PTY
   panes are actually implemented. Notable because the type
   (`SpawnRequest.isolation`) already exists, suggesting this was scoped
   and then not finished, not merely deferred.

## What this changes

Before this audit, the working understanding (from AGENTS.md) was: Job
Objects are wired and best-effort (a known, scoped gap), restricted tokens
and HCS are "real, tested" (implying wired), and the PTY supervisor gap and
malt-elevate stubs were separately known, smaller items. Tracing every call
site end to end shows a more specific and more actionable picture: exactly
one enforcement mechanism (Windows Job Objects, tier-blind) is wired into
the live spawn path at all; every other primitive that exists in
`malt-platform` — restricted tokens, HCS, and all of Linux's and macOS's
mechanisms — is either fail-closed-but-unreferenced or literally only
invoked by the capability prober. The `required`/`preferred`/`disabled`
policy work the project owner is asking for is necessary, but it should
land as part of (not instead of) actually wiring real per-tier enforcement
on Windows and adding any enforcement at all on Linux/macOS — a `required`
policy that fails closed on a mechanism that was never really providing
containment in the first place would be an improvement in honesty, not an
improvement in safety.
