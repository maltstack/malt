# Findings: Persistence & Restore Audit — Command History, Compat Restore, Scrollback, Env Round-Trip

Date: 2026-07-24
Context: Strategic hardening push. The team retired AGENTS.md's phased
Implementation Roadmap in favor of a correctness/hardening-first strategy,
scoped by a 9-step "agent + human coexist on one session" demo used only to
judge priority, not to build against. This audit covers the persistence/
restore slice of that priority list: persistent execution history and
session restoration (demo steps 7-9 in particular — session survives
disconnect, daemon restart restores history, agent resumes from the last
execution event instead of scraping the terminal).

Overlap note: a parallel audit,
`docs/findings/2026-07-24-audit-execution-correctness.md`, covers the same
`CommandBlock`/`command_id` territory from the execution-correctness angle
(data-flow: how a command's lifecycle would populate a `CommandBlock` in
memory) and explicitly flagged one question as out of scope for itself,
directed at this agent: "whether `PaneRuntime`'s in-memory `command_blocks`
ring buffer needs to be included in `build_persisted_session`/
`PersistedSession` ... for history to actually survive a daemon restart."
Section 1 below answers that question directly rather than re-deriving the
in-memory wiring analysis that document already did thoroughly. Everything
else in this document (compat-pane restore, scrollback, Env round-trip,
store hardening spot-check) is this audit's own, non-overlapping territory.

Method: read AGENTS.md (Phase B section, "What's Implemented"), BACKLOG.md,
architecture.md's Scrollback/Attach-sync/Session-persistence sections, both
existing findings docs, and ADR-0002 for calibration. Then traced every claim
through actual current source with `grep`/`Read`, file:line by file:line,
and ran the relevant test suites (`cargo test -p malt-daemon`,
`cargo test -p malt-daemon --test coordinator`, `cargo test -p mash --test
env`) to confirm claims against passing/failing tests rather than reading
code in isolation. No code was modified — read-only investigation.

## 1. Command history does not survive restore today, and would not even if `CommandBlock` were wired up

Two independent, stacked gaps, confirmed separately:

**Gap A (in-memory wiring — matches the execution-correctness audit's finding
4, re-confirmed here):** `grep -rn "CommandBlock {" --include=*.rs` and `grep
-rn "push_command_block"` across the whole workspace, excluding `tests/`,
return zero hits outside `crates/malt-session/tests/pane.rs`. `CommandBlock`
(`crates/malt-session/src/pane.rs:11-18`) and `PaneRuntime::push_command_block`
(`:69-74`) are fully built and unit-tested but never constructed or called
from `malt-daemon`. Confirmed unchanged from ADR-0002 and BACKLOG.md's
existing claims.

**Gap B (persistence schema — new, not covered by the execution-correctness
audit, which was scoped to in-memory data-flow only):** even if Gap A were
fixed and `CommandBlock`s started accumulating in a live `PaneRuntime`, they
still could not survive a daemon restart, because the wire format that
`build_persisted_session` produces has no field for them at all.
`schemas/persist/session.vexil:22-26`:

```
message PersistedPane {
    cwd       @0 : string
    title     @1 : optional<string>
    pane_type @2 : PersistedPaneType
}
```

No `command_blocks` field, no execution-history field of any kind. And the
actual construction site, `build_persisted_session`
(`crates/malt-daemon/src/executor/session_thread.rs:599-658`), only ever
reads `env.get_str("SHELL")` and `env.get_str("PWD")` out of the live session
to build the `PersistedPane` — it has no access path to `PaneRuntime`'s
`command_blocks()` at all today, independent of whether `CommandBlock` itself
gets populated upstream.

**Conclusion for demo step 9** ("agent resumes from the last execution event
instead of scraping the terminal", specifically across a daemon restart):
this is a two-part fix, not one. Wiring `CommandBlock` construction
(ADR-0002 Phase 3/4, owned by the execution-correctness thread) is necessary
but not sufficient — `schemas/persist/session.vexil`'s `PersistedPane` also
needs a bounded `command_blocks: array<CommandBlock>` (or similar) field, and
`build_persisted_session`/`restore_session` both need to read/write it. Doing
Gap A alone would make command history durable only within a single daemon
process's lifetime (survives detach/reattach while the daemon stays up, lost
on daemon restart) — worth stating explicitly since "persisted" could
otherwise be assumed to mean "survives daemon restart" when it currently
wouldn't, even after the in-memory fix lands.

`CommandBlock` itself would need a schema-compatible type (it's a plain Rust
struct in `malt-session`, not schema-generated) — either hand-mirror its
fields into a new `.vexil` message, or move the type to be schema-generated
directly, to avoid two representations drifting.

## 2. Compat-pane restore stub — confirmed still accurate; minimal correct implementation path sketched

Confirmed unchanged from BACKLOG.md/the plan-implementation audit:
`crates/malt-daemon/src/executor/coordinator.rs:547-551`:

```rust
PersistedPaneType::Compat { .. } => {
    return Err(DaemonError::RestoreFailed(
        id.clone(),
        "compat pane restore not yet implemented".to_string(),
    ));
}
```

`grep -rn "fn spawn_compat"` across the whole workspace: zero hits. The
design's restore function was never written, matching BACKLOG exactly.

**What shell-session restore does, as the template
(`crates/malt-daemon/src/executor/coordinator.rs:518-572`,
`SessionExecutor::spawn_with_cwd` at
`crates/malt-daemon/src/executor/session_thread.rs:221-264`):**

1. Read the one `PersistedPane` out of `persisted.panes` (currently always
   exactly one pane — single-pane model, see the caveat at the end of this
   section).
2. Spawn a fresh OS thread running a new `SessionExecutor`, with a fresh
   `mash::Env::from_os()`, apply session isolation
   (`apply_session_isolation`, `session_thread.rs:30-52` — creates a **new**
   Job Object, does not attempt to reattach to anything), and set `PWD` from
   the persisted `cwd`.
3. Wire the returned `(cmd_tx, thread)` into `SessionLifecycle::Active`.
4. No subprocess is re-attached to — mash itself *is* the "process" for a
   Shell pane (it's in-process), so "restore" for Shell just means
   re-creating the shell interpreter state (which itself is only partially
   restored — see Section 4) and starting fresh. There's no real child
   process continuity question for Shell panes at all.

**What `spawn_compat` would need to do, concretely, since a Compat pane's
process genuinely is external (unlike mash's in-process shell):**

1. Read `PersistedPaneType::Compat { program, args }` and the pane's `cwd`.
2. Spawn a **new** OS process (`malt_platform::process::spawn_with_pty` —
   the same function `ProcessSupervisor` already wraps for compat/legacy
   processes, per AGENTS.md's own note that this path isn't yet wired to
   isolation tiers) with `program`/`args`/`cwd`, under the session's
   isolation tier (same `apply_session_isolation`-style Job Object
   assignment shell restore already does).
3. Construct a fresh `CompatTranslator::new(cols, rows)`
   (`SessionExecutor::init_compat`, `session_thread.rs:267-269`) — there is
   no VT/grid state to restore from persistence (the architecture doc is
   explicit that "Process memory is not captured" and scrollback is
   ephemeral by design, so a blank grid is correct, not a shortcut).
4. Feed the process's stdout/stderr through the new `CompatTranslator` the
   same way the live PTY-output handler already does
   (`session_thread.rs:309-321`, the existing `PtyOutput` `SessionCommand`
   variant), so the newly-spawned process's output starts flowing into the
   grid/renderer pipeline immediately.
5. Wire the process handle into whatever bookkeeping `ProcessSupervisor`
   uses for live compat processes (kill/resize/check_exited), so a restored
   Compat pane is not a second, differently-tracked process type from a
   freshly-created one.
6. Return the same `(cmd_tx, thread)` shape `restore_session` already expects
   from the Shell branch, so the surrounding match/lifecycle-transition code
   needs no change beyond adding this arm.

This is genuinely re-launch, not re-attach — same model architecture.md
already specifies for Shell/App panes ("re-launch fresh processes"), just
never implemented for Compat. No new design decision is needed, only
implementation; the "process memory is not captured, re-launch on restore"
policy already covers this case, `spawn_compat` just needs to exist.

**Caveat surfaced while reading `restore_session` for this section, not
asked for but worth flagging:** `coordinator.rs:532`,
`persisted.panes.iter().next()`, picks the first entry of the pane `BTreeMap`
by key order — it does **not** consult `persisted.focus` (the `PaneId` the
schema explicitly carries at `schemas/persist/session.vexil:13`) to find the
*correct* pane. Harmless today because the single-pane model guarantees
exactly one entry, but this is a landmine for Phase F multi-pane work: once
sessions can persist more than one pane, restore will pick an arbitrary one
instead of the focused one (or, worse, would need restore logic for *all*
panes, not just one). Flagging now since it's directly adjacent to the code
this section already had open, cheap to note, expensive to rediscover later.

## 3. Scrollback — confirmed 100% unimplemented, schema-only

Grepped for every angle architecture.md's scrollback design would touch:

- **mmap usage anywhere in the daemon/compat/renderer path:** only hit in
  the whole `crates/` tree is `crates/malt-platform/src/isolation/seccomp.rs`
  (an unrelated `mmap` syscall-filtering entry for the seccomp BPF isolation
  tier) — zero mmap-backed scrollback log exists.
- **`ScrollbackRequest`/`ScrollbackResponse` handling:** the schema messages
  exist (`schemas/render.vexil:115-131`, `@type(0x06)`/`@type(0x07)`) and the
  generated wire constants exist (`crates/malt-protocol/src/codec.rs:71-72`,
  `MSG_SCROLLBACK_REQUEST`/`MSG_SCROLLBACK_RESPONSE`). But the VNP listener's
  dispatch match (`crates/malt-daemon/src/vnp_listener.rs:367-412`) only
  handles `MSG_KEY_EVENT`, `MSG_RESIZE`, and `MSG_FRAME_ACK` — confirming
  AGENTS.md's own "KeyEvent/Resize/FrameAck inbound" claim is complete and
  accurate, and scrollback genuinely has no inbound handler at all. Grepping
  the constants' usage sitewide finds them only in the codec's own test file
  (`crates/malt-protocol/tests/codec.rs:117-118`, asserting the constant
  values) and a doc comment in `priority.rs:62` classifying their priority
  tier — no handler, anywhere.
- **A `scrollback` module:** `find crates -iname "*scrollback*"` returns
  nothing.
- **Everywhere else "scrollback" appears in code** (not docs): a
  `scrollback_lines` config knob (`crates/malt-config/src/decode.rs`,
  `build.rs:66`, default `10000` — matches architecture.md's stated default,
  parsed and tested, but read by nothing downstream), a thread-pool doc
  comment ("Thread pool for disk I/O (persistence, scrollback)",
  `crates/malt-daemon/src/executor/pools.rs:8` — comment only, no scrollback
  I/O exists), and one explicit acknowledgment in the renderer itself:
  `crates/malt-renderer/src/walker.rs:191`, `// Pass through — scrollback
  deferred.`

**Confirmed: genuinely 100% unimplemented, not partially started anywhere
unexpected.** The only real artifact is a parsed-and-defaulted config value
nothing reads.

**Direct connection to the demo, stated explicitly per the task's framing:**
architecture.md's own restore-behavior section
(`docs/design/architecture.md:1690-1692`) states scrollback is intentionally
ephemeral by design ("Scrollback from the previous session is gone
(ephemeral). This is the same model as tmux session restore") — so
scrollback's absence is *not* itself a bug relative to the current design.
But that same design intent means scrollback was never meant to be the
mechanism for demo step 9 in the first place — `CommandBlock`-based
persistent history (Section 1) is architecturally the only path to
"resume from the last execution event," and it has its own two-part gap.
Today, with **both** scrollback and `CommandBlock` persistence absent, an
agent that reattaches to a long-running session (or a daemon that restarts
mid-session) has literally no mechanism to recover anything about what
happened before it (re)connected beyond the live/current terminal grid
snapshot `RegisterVnpClient` hands a newly-attaching client
(itself only "most recent chunk," not full current-screen state, per the P0
follow-on gap already logged in BACKLOG.md). This is the sharpest concrete
argument for why Section 1's Gap A + Gap B is the highest-leverage fix in
this whole area — it's the only one of the two "history" mechanisms the
architecture actually commits to persisting at all.

## 4. Daemon restart round-trip for shell sessions — cwd survives; most other shell state is silently dropped

Confirmed via `cargo test -p malt-daemon --test coordinator` (26 tests, all
passing — matches BACKLOG's "26 tests" claim exactly) that restore itself
works mechanically. But tracing exactly what `build_persisted_session`
captures versus what a live mash `Env` actually holds surfaces a real,
concrete round-trip gap:

**What `build_persisted_session` captures
(`crates/malt-daemon/src/executor/session_thread.rs:599-658`):** exactly two
scalar strings read via `env.get_str(...)` — `SHELL` (used to populate
`PersistedPaneType::Shell { shell_path }`) and `PWD` (used for
`PersistedPane.cwd`). Nothing else.

**What a live mash `Env` actually holds
(`crates/mash/src/env.rs:276-390`):** a variable scope stack (exported *and*
non-exported variables), `functions: HashMap<String, FunctionDef>`,
`aliases: HashMap<String, String>`, `jobs: Arc<Mutex<Vec<JobEntry>>>`, shell
options, a directory stack, traps, and more — none of which
`build_persisted_session` touches.

**What `restore_session`/`spawn_with_cwd` reconstructs on restore
(`crates/malt-daemon/src/executor/coordinator.rs:518-572`,
`crates/malt-daemon/src/executor/session_thread.rs:221-264`):** a brand-new
`Env::from_os()` (fresh OS-inherited environment, not the prior session's
variables), with only `PWD` explicitly overwritten from the persisted `cwd`
(`session_thread.rs:236-239`). **`shell_path`, the one other field actually
captured in `PersistedPaneType::Shell`, is never read back at all** —
`spawn_with_cwd`'s match arm on `PersistedPaneType::Shell { .. }` discards
the field with `..` (`coordinator.rs:540-543`); the daemon captures it on
every persist cycle and then throws it away on every restore.

**Concretely, what survives a daemon restart today vs. what's silently
lost, for a shell session:**

| State | Survives restart? |
|---|---|
| Working directory (`cwd`/`PWD`) | Yes — explicitly restored |
| Isolation tier | Yes — `IsolationTier` is part of `PersistedSession` and a fresh Job Object is created under it |
| `SHELL` env var value | **No** — captured on persist, discarded on restore (dead round-trip) |
| Any other exported variable (`export FOO=bar`) | **No** — fresh `Env::from_os()` only has the *daemon process's* current OS environment, not what the session had |
| Non-exported shell variables | **No** — never captured at all |
| Shell functions defined interactively | **No** — never captured at all |
| Aliases | **No** — never captured at all |
| Job control state (backgrounded jobs, `jobs` list) | **No** — never captured at all; also moot given the separately-tracked backgrounded-command survival bug in BACKLOG.md P1 |
| Directory stack (`pushd`/`popd`) | **No** — never captured at all |
| Traps | **No** — never captured at all |

**The sharpest finding here: this is not a fundamental limitation — the
mechanism to do all of this already exists, built and tested, and is simply
never called.** `mash::Env::to_snapshot()`/`apply_snapshot()`
(`crates/mash/src/env.rs:1150-1176`, `:1178+`) produce/consume an
`EnvSnapshot` (`:1524-1533`) that captures exactly the fields listed as
"No" above — `variables` (the full global scope, not just two scalars),
`options`, `aliases`, `functions` (re-parsed from stored source text on
apply), `dir_stack`, `cwd`, and `traps`. It is exercised by real, passing
tests (`cargo test -p mash --test env` — 34 tests passing, including the
snapshot round-trip tests under `// Task 5: Persistence (EnvSnapshot)` in
`crates/mash/tests/env.rs:302+`). `grep -rn "to_snapshot\|apply_snapshot"
crates/malt-daemon` returns zero hits — `malt-daemon` has never called
either method. The original design doc
(`docs/design/legacy-specs/phase2-mash-env.md:318`) is explicit about the
intent this was built for: *"Session-scoped persistence: when you `export
FOO=bar` in session 3, it's stored in the session's `PersistedSession` data.
On detach/reattach or daemon restart, the EnvSnapshot is restored and
`FOO=bar` is back."* That never happened on the daemon side — this is the
same shape of gap as `CommandBlock` (Section 1, Gap A): a fully-built,
tested subsystem with zero callers outside its own crate's tests.

Closing this gap needs two things, same as Section 1: (a) call
`env.to_snapshot()` in `build_persisted_session` and store the result, and
(b) extend `PersistedPane`/`PersistedSession`
(`schemas/persist/session.vexil`) with a field to actually carry it — today
the schema has no room for an `EnvSnapshot` any more than it has room for
`CommandBlock`s. Both gaps are schema-shaped, not just code-shaped.

## 5. Session-store hardening claims — spot-checked, accurate

All five AGENTS.md claims verified directly against
`crates/malt-daemon/src/store/{mod.rs,debounce.rs}` and confirmed by test
runs (`cargo test -p malt-daemon` — full crate green):

- **`.bak` backup:** `atomic_write` (`store/mod.rs:156-165`) copies the
  existing file to `{path}.bak` before the temp-file rename, whenever a
  prior file exists. Confirmed by `atomic_write_creates_bak`
  (`crates/malt-daemon/tests/store.rs:348+`), which checks the `.bak`
  actually decodes to the *first* save's content after a second save — this
  is a real behavioral test, not just a path-exists check.
- **Corruption quarantine:** `unpack_from_bytes` (`store/mod.rs:115-154`)
  renames any file that fails `Unpack` to `{stem}.corrupt.{unix_ts}.vxb` in
  the same directory, best-effort (falls back to leaving the file in place
  and logging a warning if the rename itself fails, rather than panicking).
  Confirmed by `corrupt_file_quarantined`
  (`crates/malt-daemon/tests/store.rs:380+`). One drift from
  architecture.md worth noting: the design doc
  (`docs/design/architecture.md:1761`, `:1806-1812`) specifies a separate
  `corrupted/` **subdirectory** with a 50-file cap and oldest-first
  eviction; the actual implementation quarantines in-place with a
  `.corrupt.{ts}.vxb` suffix and has **no cap or eviction** — a corrupted-
  file-generating bug could accumulate quarantine files unboundedly. Not a
  correctness bug (corruption handling itself works), but a real gap
  against the documented design, cheap to fix, and independent of anything
  else in this report.
- **Debounced 1s flush:** `debounce.rs`'s background thread flushes any
  session/daemon-state entry dirty for ≥1s (`threshold =
  Duration::from_secs(1)`, `:110`, checked every 100ms poll,
  `:111`/`:136-171`). Confirmed by `flushes_after_one_second_idle`
  (`debounce.rs:228-239`, sleeps 1.2s and checks the write landed) and
  `flush_all_is_immediate` (`:192-211`, confirms the shutdown-path bypass
  completes in <500ms rather than waiting on the timer).
- **XDG-compliant data dir:** `crates/malt-bin/src/daemon.rs:15-17` calls
  `malt_config::paths::data_dir()` and passes it directly into
  `SessionStore::new`. Confirmed wired at the one real daemon-startup call
  site (`malt-daemon`'s own tests all use `tempfile::tempdir()` instead, as
  expected for test isolation).
- **Counter restore on startup:** `Coordinator::new`
  (`crates/malt-daemon/src/executor/coordinator.rs:53-90`) reads
  `next_session_id`/`next_pane_id` from `DaemonState` via
  `store.load_daemon_state()` before falling back to `1u32` defaults, and
  logs the restored values. This is the same code path Section 1/4 already
  traced for session restoration, so it was re-confirmed rather than
  separately re-derived.

All five claims hold. The one drift found (corrupted-file directory/cap) is
minor and independent of the corruption-handling correctness itself.

## Prioritized recommendations

1. **Wire `mash::Env::to_snapshot()`/`apply_snapshot()` into
   `build_persisted_session`/session restore, and extend
   `schemas/persist/session.vexil` to carry an `EnvSnapshot`-shaped field.**
   Severity: **High** — this is silent, surprising data loss (a user's
   `export`s, aliases, and shell functions vanish on every daemon restart or
   detach/reattach, with no error or warning) against a mechanism that is
   already built, tested, and was explicitly designed for exactly this.
   Effort: **Medium** — the hard part (capture/restore logic) already
   exists and is tested in `mash`; the work is a schema field, threading
   `to_snapshot()`'s result through `build_persisted_session`, and calling
   `apply_snapshot()` after `Env::from_os()` in `spawn_with_cwd`/`spawn`.
   Unblocks: demo step 7 ("session survives client disconnection") only
   partially works today for shell state — the process survives, but a
   restart drops most of the shell's memory. Also directly relevant to step
   9's "resume" framing, since an agent's exported context (env vars set up
   for a task) is exactly the kind of state that should survive.

2. **Add a `command_blocks`-equivalent field to `PersistedPane` in
   `schemas/persist/session.vexil`, and wire `build_persisted_session`/
   restore to read/write it, in the same change as (or immediately
   following) the execution-correctness audit's `CommandBlock`
   construction fix.** Severity: **High** — directly blocks demo step 9 as
   stated; without both halves (construction + persistence), "resume from
   last execution event" cannot survive a daemon restart even after the
   execution-correctness fixes land. Effort: **Medium-High** — depends on
   the execution-correctness thread's construction-side work landing first
   (Section 1 spells out both are needed; doing this half alone has nothing
   to persist yet). Recommend sequencing as one coordinated piece of work
   across both audits rather than two independent tickets. Unblocks: demo
   step 9 fully, step 8 partially (mid-session history for an attaching
   human/agent).

3. **Implement `spawn_compat` per the sketch in Section 2.** Severity:
   **Medium** — real, confirmed gap, but narrower blast radius than 1/2
   (only affects Compat-typed panes, which aren't yet the primary session
   type in practice — Shell sessions are). Effort: **Medium** — no new
   design needed; re-launch-not-reattach is already the documented policy,
   the shell-session restore path is a working template, and
   `ProcessSupervisor`/`spawn_with_pty` already exist to spawn the process.
   Unblocks: demo step 7/9 for any session using a Compat pane specifically
   (e.g., an agent running a long build via a real subprocess rather than
   mash's in-process execution).

4. **Fix the dead `shell_path` round-trip** (`coordinator.rs:540-543`
   discards the field `session_thread.rs:606-620` spends effort computing
   and persisting). Severity: **Low** — cosmetic today since `SHELL` rarely
   changes mid-session, but it's a one-line contradiction (captured, never
   used) worth closing while already touching this code for item 1.
   Effort: **Trivial.** Unblocks: nothing on its own, but closes an
   inconsistency directly adjacent to item 1's changes.

5. **Add a bound + LRU eviction to the corruption-quarantine directory**,
   matching architecture.md's documented 50-file cap (currently unbounded).
   Severity: **Low** — only matters under a recurring serialization bug,
   which is itself rare, but unbounded disk growth from quarantined files is
   an easy, cheap fix. Effort: **Trivial.** Unblocks: nothing demo-related;
   pure hardening.

6. **Fix `restore_session`'s pane selection to use `persisted.focus` instead
   of `panes.iter().next()`** (Section 2's caveat). Severity: **Low today**
   (harmless under the current single-pane model) but flagged now because
   it's cheap to fix immediately and expensive to rediscover once Phase F
   multi-pane work lands. Effort: **Trivial.** Unblocks: nothing today;
   pre-empts a Phase F regression.
