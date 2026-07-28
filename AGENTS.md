# AGENTS.md

Instructions for AI agents (and anyone else) working in this repo. Claude
Code loads this via the `@AGENTS.md` import in `CLAUDE.md` — see that file
if you're looking for why there are two files; there aren't, really.

**Read order if you are new here:** *Start Here* for what this is, then
*Non-Negotiables*, then *How Work Goes Wrong Here* — that third section is the
one that will actually save you time, because every entry in it cost someone a
day.

---

## Start Here

MALT (structured terminal platform) inverts the traditional terminal model:
**the daemon is the authority, not the renderer.** The daemon owns session
state, layout, pane identity, and structured output. Clients are
interchangeable consumers of a typed `RenderCommand` stream. All
inter-component communication uses VNP (Vexil Native Protocol) — typed,
schema-defined, bitpack-encoded.

19 Rust crates + 1 SvelteKit web client. `malt` starts a daemon with an
in-process POSIX shell (`mash`), serves HTTP + VNP socket APIs, and opens an
interactive TUI.

**Do not trust any status claim in this file, including this one, without
re-running the gates.** Numbers here drift; the suite does not. See *Session
ritual* below.

### Where to look for what

| Question | Answer lives in |
|---|---|
| What is the target design? | `docs/design/architecture.md` — target state, **not** a status tracker |
| Why was this decided? | `docs/adr/` — check before re-deciding anything |
| How do we know it works? | `docs/findings/` — dated evidence from actually running things |
| What should I work on? | `docs/BACKLOG.md` (prioritized) and `docs/briefs/` (scoped, verified) |
| What shape does the remaining work have? | `docs/ROADMAP.md` — a map by domain, **not** an order |
| What was built, and why, per feature? | `specs/NNN-*/` |
| What exists today, subsystem by subsystem? | `docs/implemented.md` — dated; re-verify before relying on it |
| What happened before 2026-07-24? | `docs/history/pre-2026-07-24.md` |

### Sibling projects

```
orix/vexil-lang/    Vexil schema language compiler + runtime — actively developed, MALT's build dependency
orix/malt/          This repo — MALT terminal platform
orix/vexil-v2/      Legacy prototype — reference only, deprecated donor for the carboy extraction
orix/malt-stack/    Sibling substrate project (Carboy/Keg/Kettle/Hops/Cask/Tap) — do NOT depend on.
                    MALT briefly depended on carboy/keg (2026-04-11, uncommitted); reverted 2026-07-24.
                    Full reverted state preserved on branch checkpoint/pre-carboy-revert-2026-07-24.
                    If porting anything from carboy-isolation (it has broader OS coverage than
                    malt-platform::isolation — AppContainer/HCS/CRIU/capability-probing), vendor the
                    code in as owned source. Never add a path/git dependency on malt-stack.
```

MALT depends on `vexil-lang` for schema compilation and `vexil-runtime` for encode/decode. The `vexil-store` crate (in vexil-lang workspace) provides `.vx`/`.vxb` persistence formats.

`~/projects/vexil-v2/` is a legacy prototype. Code quality and engineering standards are poor — do not port code verbatim. It does contain working implementations of systems not yet built in MALT (isolation, scrollback, plugin lifecycle, etc.) that can be used as **reference for logic and algorithms**. When implementing a Phase B–I feature, check vexil-v2 for a working equivalent first.

---

## Non-Negotiables

**No time constraints. No deadlines. Every line of code is production-ready
and mission-critical.**

- No stubs, no `todo!()`, no `unimplemented!()`, no placeholder logic
- No hand-waving ("this could be improved later", "for now we just...")
- No deferring — if a component is needed, implement it fully
- No shortcuts that would need to be revisited
- No half-measures: if the architecture specifies X, implement X completely
- All error paths handled — no silent ignores, no bare `unwrap()` outside tests
- All tests pass before any task is marked complete

This is the standard for every change, regardless of scope.

### Hard invariants

These are non-negotiable. Violating any is a bug. Also captured, with the
day's process lessons, in `.specify/memory/constitution.md`.

These are non-negotiable. Violating any is a bug. (Also captured, with the
day's process lessons added, in `.specify/memory/constitution.md`.)

1. **VT codes in `malt-compat` only.** No other crate may import `vte` or handle escape sequences. ✅ Clean.
2. **OS calls in `malt-platform` only.** No `nix`, `windows-sys`, `libc`, `std::os::unix` elsewhere. ✅ Clean — `PermissionsExt` violation in `mash/src/executor.rs` fixed in Phase A.
3. **`malt-protocol` is dependency-free within workspace.** Only external deps. ✅ Clean.
4. **`malt-plugin-sdk` has zero internal deps.** ✅ Clean — only external deps (wasmtime, serde, thiserror).
5. **All `unsafe` blocks require `// SAFETY:` comments.** ✅ Clean — enforced throughout, including the 2026-07-24 HCS port (27 unsafe blocks, all documented).
6. **No `unwrap()` or `expect()` in non-test code.** ✅ Clean — all found uses are in `#[cfg(test)]` blocks.
7. **VNP is the only inter-component protocol.** ✅ Clean. Full bitpack envelope used post-handshake for all message types.
8. **Shell ships when POSIX conformance suite passes.** Smoosh (183/183 Windows, 186/186 WSL) + Modernish. Not yet wired to CI. Fix in Phase C.
9. **Layer violations are compile errors.** No upward dependencies in the crate graph. ✅ Clean.
10. **Invariants are CI-enforced via `deny.toml`.** ✅ Clean — `deny.toml` added in Phase A.
11. **Vendor, never depend on unstable siblings.** Added 2026-07-24 (ADR-0001). If something from malt-stack or another sibling project is useful, port it in as owned source — never a live path/git dependency.
12. **No silent scope-jumps; commit at real checkpoints.** Added 2026-07-24. If a task starts looking like it needs a bigger rethink, write that down (an ADR draft, a backlog item) instead of pivoting into it mid-task.

---

## How Work Goes Wrong Here

Five recurring failure classes, each learned the expensive way. They are
listed together because they are easy to confuse and their fixes differ.

### 1. It exists, it is tested, and nothing calls it

**The most common defect in this repo is not a missing mechanism.** Six
instances, each found only after someone started building a replacement:

| What existed | What was missing |
|---|---|
| `TokenStore`/`AuthContext`/`RateLimiter`, fully unit-tested | never wired into `build_router` — every route open |
| `AuthorityTracker`, complete with tests | driven only by `AttachClient`, which only a *test* ever sent |
| `InputClaim`/`InputAuthorityChanged` schema + codec constants | no handler anywhere |
| `OutputChunk` in `schemas/shell.vexil`, doc saying "MASH sets it at emission time" | nothing ever emitted one |
| 12 of 14 `malt-platform::isolation` modules, 13–17 tests each | zero callers outside their own crate |
| `malt-elevate`'s ten privileged operations | nine returned `stub_success` — success for work never done |

So a survey answering *"does this exist?"* is the wrong survey. It returned
"no, build it" six times when the answer was "yes, wire it."

**A survey must establish, in this order:**

1. **Does it exist?** Search schemas, codec constants and type definitions —
   not just function names. `OutputChunk` was found only by reading
   `schemas/`, with its `@type` constant already allocated.
2. **Is it called from production code?** Grep for callers, **excluding the
   defining crate and all test files.** This is the step that gets skipped.
3. **Is that caller reachable?** `AttachClient` had exactly one sender and it
   was a test, which made the tracker look wired for months.
4. **Do its tests exercise behaviour, or construct types?** See class 4.
5. **Only then** check sibling projects for a working equivalent.

**Record the survey.** If it changes the shape of the work it belongs in
`docs/findings/` before planning proceeds — see
`docs/findings/2026-07-26-isolation-prior-art-survey.md`, including its "what
this survey did not establish" section. A survey that records only what was
found invites the next person to over-trust it.

### 2. Wired backwards

Rarer than class 1 and harder to catch, because **every reachability check
passes.** The mechanism exists, is called, and is reachable — and is wrong.

`docs/briefs/007`: the Unix PTY drops the slave fd and hands the child dups of
the *master*, so nothing holds the slave and the first read returns `EIO` with
zero bytes. Unix compat panes have never delivered output. The code says what
it does in a comment, and the comment reads as deliberate.

**When a subsystem has never been exercised on a platform, "it compiles and
has callers" tells you nothing.** Run it.

### 3. A value that re-derives its own truth

**Nothing may re-derive at use time what was decided at creation time.**

| What was re-derived | From what | Result |
|---|---|---|
| A session's isolation level | reconstructed separately by each surface | create, list and query could disagree — spec 007 |
| How a session is contained | two `Env` fields, one inert, one working | reported vs actual containment could drift — spec 008 |
| Whether an HCS handle is fake or native | the `MALT_HCS_FAKE` env var, read *again* at wait/close | a fake handle reached the native API and faulted the process — `docs/findings/2026-07-27-elevate-build-lock-and-teardown.md` |

The third is sharpest: the global was correct when the handle was created and
wrong when it was used. Nothing was concurrent-unsafe in the usual sense — the
answer simply expired. **A branch on mutable global state is a decision with
no expiry date, and the value it governs does not know when it went stale.**

Every fix had one shape: **the value carries its own provenance.** One
isolation status every surface reads; one carrier conveying containment; a
handle that remembers the backend that made it.

**When reviewing:** if a function decides *what kind of thing* its argument is
by consulting configuration, an env var, a feature flag or a global registry,
ask why that fact is not travelling with the argument. "It was set correctly
when we started" is the defect, stated aloud.

### 4. Tests that pass by construction

Test *count* is not evidence of function.

Two real `job_objects.rs` bugs survived months of green tests — an undersized
`IO_COUNTERS` struct silently failing every job creation, and a hardcoded
active-process-limit of 1 silently rejecting the second process in any job —
because the tests built structs instead of calling Win32. Both were found the
day someone wrote a test that made the real call.

**If a file's tests only check `Send`/`Debug` bounds or construct types
directly, that is a reason to add a test that calls the real thing — not
evidence the code works.** 54 isolation tests passing in 0.01 s is what this
looks like from the outside.

### 5. Shared process state, and where verification happens

**When a test flakes, suspect a different test.** Four instances:

- `mash/tests/executor.rs` — a test called `set_current_dir` without holding
  `CWD_LOCK`. It never failed itself; it yanked the CWD out from under
  whichever lock-holding test was running, so the failures surfaced in
  `heredoc_redirect_feeds_stdin_to_cat` and
  `exec_input_redirect_registers_readable_shell_fd` instead. It looked like
  three independent flaky tests for months.
- `CWD_LOCK` had 9 of 19 call sites using poison-fragile `.lock().unwrap()`,
  so one panic cascaded into unrelated failures. Use
  `.lock().unwrap_or_else(|e| e.into_inner())`.
- `malt-platform/src/fs.rs`'s `MALT_SESSION_ID` env-var tests, same pattern.
- An unjoined HCS reaper thread read an env var after the test that set it had
  already finished — which is class 3 as well.

If a test mutates shared process state (CWD, env vars, global statics) without
holding a lock for its whole duration, treat that as a bug the moment anything
flakes.

**`thread::sleep` cannot establish a precondition**, only make a race likely to
resolve one way. Four daemon tests used `sleep(50ms)` to mean "the first
command has started"; under parallel load it hadn't, and `gateway_backend.rs`
failed 3 runs in 5. Wait on observable state —
`Coordinator::execution_queue_state` exists for this. Note `active` is set by
the worker *before* the control actor records history, so waiting on `active`
is not sufficient when the assertion is about history.

**Where you verify changes what you can see.** An unjoined thread faulting at
teardown reproduced **100% under agent harnesses and never once in a human
terminal** — console I/O is slow enough to close the timing window. The two
environments that run *without* a console are **CI and the helper service** —
which is to say, production.

So a suite that passes in a terminal has not been tested in the context it
ships into, and **when only automation reproduces a crash, suspect the code
before the automation.** Most of 2026-07-27 went the other way.

---

## Working Practice

### Session ritual

This project has been abandoned-via-rewrite three times (vexil-v2 → malt →
malt-stack) after multi-day uncommitted sprints. Two habits prevent a fourth:

1. **Rebuild + retest before resuming after any gap.** Don't trust a status
   doc — including this one. Run the gates below.
2. **Commit at real checkpoints, not multi-day piles.** If a change feels too
   big to commit, that is a signal to commit sooner, not later.

### No silent scope-jumps

If something looks like it needs a bigger rethink mid-task, write it down — an
ADR draft, a backlog entry, a brief — instead of pivoting into it. A
scope-jump written down is a proposal; one absorbed mid-feature is how this
project was abandoned three times. (Constitution IX.)

### Feature work goes through Spec Kit

`/speckit-specify` → `/speckit-plan` → `/speckit-tasks` → `/speckit-implement`,
producing `specs/NNN-feature/`. The `malt` preset appends this repo's standing
rules to each artifact. Use `/speckit-malt-brief` for work that is smaller
than a feature but larger than a backlog line.

---

## Build, Test, Verify

### The gates

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

**`vexilc` must be on PATH.** `malt-protocol`'s `build.rs` shells out to it,
so *nothing* in the workspace compiles without it — and it fails as
`failed to run vexilc: No such file or directory` from a build script, which
reads like a code error rather than a missing prerequisite. CI installs a
pinned revision (`.github/actions/setup-vexilc`); pin the same one locally:

```bash
cargo install --git https://github.com/vexil-lang/vexil     --rev fc8c51f31f1f25f0b2885fc98696ad1c5ee543c7 vexilc
```

Pinned rather than latest on purpose: a different `vexilc` can emit different
generated code, so an unpinned local build compiles something other than what
CI compiles.

**Run clippy with `-D warnings`.** Plain `cargo clippy` is not the gate: a
warn-level lint (`items_after_test_module`) sat in `gateway_backend.rs` while
local runs looked clean, failed CI's blocking job, and — because clippy runs
before tests — meant **the test suite had not run in CI for ~110 commits.**

Smoosh POSIX conformance is a gate whenever `mash` or `malt-tools` changes.
When it does not apply, say so explicitly, so its absence is not read as an
oversight.

```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 183, skipped unsupported: 3

cargo test -p mash --test executor
# Expected: 228 passed, 0 failed
```

### Build parallelism

`~/.cargo/config.toml` on this machine caps `[build] jobs = 6` on a 16-core
host. **Not a repo setting** — a machine preference, deliberately kept out of
the repo so it never reaches CI or another developer's core count.

The cap exists for concurrent work across *several* projects, not for one
build. A single build using 14 of 16 cores is fine; three cargo invocations
each assuming they own the machine is 48 rustc processes on 16 cores, and
that oversubscription is what makes the desktop stutter.

Uncap per-run when a long build is the only thing happening — no edit needed:

```bash
cargo build -j 16
cargo test -j 16
```

**`cargo test` parallelism is separate and cargo has no setting for it**: test
binaries run one thread per core *on top of* build jobs. Cap with
`-- --test-threads=N` or `RUST_TEST_THREADS`. It matters here because supervisor
and Smoosh tests spawn real processes.

WSL is capped independently in `~/.wslconfig` (`processors=8`), so a slow
Linux build is not the same problem as a stuttering desktop.

### Test conventions

- `tempfile::tempdir()` for anything touching the filesystem.
- VNP listener tests bind random ports (`bind("127.0.0.1:0")`).
- Supervisor tests spawn real processes (`echo`, `sleep`/`ping`).
- Gateway route tests use a mock backend via `tower::ServiceExt::oneshot`.
- Plugin SDK tests build minimal WASM with the `wat` crate.
- No test requires a GPU — `maltty` is build-verified only.
- A test that cannot run truthfully on this host **skips loudly with the
  reason**. It never passes quietly, and it is never deleted to make a suite
  green (`docs/briefs/007` is `#[ignore]`d with its reason, not `#[cfg]`-ed
  out).

### Linux: use the WSL mirror

```bash
bash scripts/wsl-mirror.sh                          # sync HEAD, build, test workspace
bash scripts/wsl-mirror.sh -- -p malt-daemon --test coordinator
```

Keeps a real git clone on the **Linux** filesystem (`~/malt`) and builds into
`/tmp/malt-build`, because building from `/mnt/c` goes through 9p and hits the
NTFS caching problem above. Sync is fetch + hard reset from the Windows
checkout, so the mirror is disposable — **never edit or commit there.** It
resolves worktree `.git` files (whose `gitdir:` is a Windows path) so it works
from a worktree as well as the main checkout.

**Why it matters, not just that it is faster.** Three cross-platform defects
were found on 2026-07-27 only once Linux tests could actually run, and the
inverted Unix PTY (`docs/briefs/007`) was root-caused in a handful of
50-second cycles. Through CI that is ~3 minutes per bit of information, and
CI's Linux job is advisory — so nothing forces anyone to look. **If a change
touches `malt-platform`, the daemon's process/PTY paths, or anything
`#[cfg]`-gated, run it here before pushing.**

### NTFS caching (critical)

On WSL with NTFS-backed repo paths, cargo caching is unreliable due to NTFS mtime granularity.
Use `CARGO_TARGET_DIR=/tmp/malt-build` for builds on WSL, or `cargo clean -p mash` followed by rebuild on Windows.
Stale binary symptoms: test failures that don't match expected behavior from source changes.

### Binary paths

- **Repo build (default):** `target/debug/mash`
- **WSL build:** `/tmp/malt-build/debug/mash`

### CI

`.github/workflows/ci.yml`. **Blocking:** Gates (Windows) and Smoosh (Windows,
path-filtered to `crates/mash|malt-tools`). **Advisory:** cross-platform
build+test and isolation-capability reporting on Windows/Linux/macOS.

Advisory means nothing forces anyone to look — so look. Three cross-platform
defects surfaced on 2026-07-27 the first time Linux tests actually ran.

**"Last known green" is `gh run list --branch main`, not a list in this file.**
A hardcoded checkpoint list was kept here until 2026-07-28 and was stale
within days of every entry.

---

## Repo Map

```
specs/                        # GitHub Spec Kit territory (adopted 2026-07-24) — per-feature
                               # specs/NNN-feature-name/{spec,plan,tasks}.md, created by
                               # /speckit-specify (Claude) or $speckit-specify (Codex).
                               # Empty until the first feature uses it.
.specify/                     # Spec Kit config, templates, constitution (.specify/memory/constitution.md)
.claude/skills/speckit-*/     # Spec Kit's Claude Code skills — /speckit-constitution,
                               # /speckit-specify, /speckit-plan, /speckit-tasks, /speckit-implement,
                               # /speckit-converge, plus optional clarify/analyze/checklist/taskstoissues
.agents/skills/speckit-*/     # The same Spec Kit workflows for Codex — invoke as
                               # $speckit-constitution, $speckit-specify, $speckit-plan,
                               # $speckit-tasks, $speckit-implement, and $speckit-converge.
docs/
  design/
    architecture.md            # Single source of truth (~2,380 lines) — moved from specs/
                                # 2026-07-24 to free that path for Spec Kit
    legacy-specs/               # The old specs/ directory's phase0-2 build specs — historical,
                                # point-in-time, not living docs. Reference only.
  adr/                         # Architecture decisions and their reasoning — check before
                                # re-deciding something already settled (e.g. ADR-0001: malt-stack)
  findings/                    # Dated evidence from actually running/testing the product —
                                # not conclusions, the "how do we know this" record
  BACKLOG.md                   # Living, prioritized "what's next and why" — check before
                                # picking up new work
  briefs/                      # Actionable work briefs — between a backlog line and a
                               # Spec Kit feature: what to do, what done looks like, and
                               # what will go wrong. Every claim verified against code at
                               # a stated date. See docs/briefs/README.md
docs/superpowers/              # DEPRECATED 2026-07-24 — no longer used, no new content goes here.
                                # Historical specs/plans left as point-in-time record.
plans/                          # RETIRED 2026-07-24 — original Phase 0-2 implementation plans,
                                # historical record. Audited against current code, see
                                # docs/findings/2026-07-24-plan-implementation-audit.md.
crates/
  malt-protocol/               # L0: VNP types, framing, envelope, codec (60 tests)
  malt-platform/               # L0: PTY, process, signals, sockets, isolation (78 tests; session
                                # isolation now wired to real Job Object containment as of 2026-07-24
                                # — see ADR-0001 and docs/findings/)
  malt-config/                 # L0: Config loading, VxDecoder, real .vx parsing (17 tests)
  mash/                        # L1: POSIX shell — lexer, parser, expander, executor (600+ tests, plus 183/183 Smoosh POSIX conformance on native Windows)
  malt-term/                   # L1: Line editor — vi/emacs, completion, history (41 tests)
  malt-tools/                  # L1: In-process POSIX utilities (80 tests)
  malt-layout/                 # L1: Layout engine — n-ary tree, resolution, focus (48 tests)
  malt-session/                # L1: Session lifecycle, pane runtime, groups (23 tests)
  malt-elevate/                # Elevated Windows service helper: VNP named-pipe lifecycle and
                                # honest operation outcomes. Privileged operations stay refused
                                # until helper-owned session entitlement validation exists.
  malt-daemon/                 # L2: Daemon core (126 tests)
  malt-compat/                 # L2: VT emulator — vte 0.15 parser + grid (26 tests)
  malt-renderer/               # L2: Renderer host — walker, dirty, client state (29 tests;
                                # known cursor-position "staircase" render bug, see docs/BACKLOG.md P0)
  malt-gateway/                # L2: HTTP API — axum, auth, rate limiting (22 tests)
  malt-plugin-sdk/             # L3: WASM plugin host — wasmtime, fuel budgets (5 tests)
  malt-mcp/                    # L3: MCP server for AI agents (6 tests)
  malt-bin/                    # L3: CLI entry point — `malt` command (9 tests; no --isolation flag yet, see docs/BACKLOG.md)
  malt-tui/                    # Client: TUI terminal — ratatui + crossterm (12 tests)
  maltty/                      # Client: GPU terminal — wgpu + winit (scaffold)
clients/
  malt-web/                    # Client: Browser terminal — SvelteKit MVP
schemas/
  *.vexil                      # VNP message schemas (15 files)
  persist/                     # Persistence schemas
```

### Crate architecture (strict layering — no upward deps)

```
L0  malt-platform     OS abstractions: PTY, process, signals, sockets, spawn_with_pty, isolation
L0  malt-config       Vexil Store config: typed structs
L0  malt-protocol     VNP spine: all shared types, framing, envelope, codec (domain/type constants, make_envelope), encode/decode

L1  mash              POSIX shell: lexer, parser, expander, executor, builtins
L1  malt-term         Line editor: vi/emacs, completion, history, multiline
L1  malt-tools        In-process POSIX utilities (cat, env, which, grep, wc)
L1  malt-layout       Layout engine: n-ary LayoutNode tree, resolution, directional focus
L1  malt-session      Session lifecycle, command block ring buffer, group management

L2  malt-compat       VT emulator (vte 0.15) — ONLY VT code in entire system
L2  malt-renderer     Renderer Host: FrameElement walker → RenderCommand emission
L2  malt-daemon       Daemon core: message bus, session-sharded executor, session store,
                      process supervisor, VNP listener, gateway backend, mash integration
L2  malt-gateway      HTTP REST API: axum routes, auth, rate limiting, shadow tree

L3  malt-plugin-sdk   WASM plugin host: wasmtime, fuel budgets, manifests (zero internal deps)
L3  malt-mcp          MCP server: AI agent tools over stdio JSON-RPC
L3  malt-bin          CLI entry point (`malt` command): clap + reqwest

    malt-elevate      Elevated helper (standalone binary, outside layer system)

Clients:
    malt-tui           TUI terminal: ratatui 0.30 + crossterm 0.29
    maltty             GPU terminal: wgpu 24 + winit 0.30 + cosmic-text 0.12
    malt-web           Browser terminal: SvelteKit 2 + Svelte 5
```

### CLI commands

```
malt                          # Auto: start daemon → create session → attach TUI
malt daemon [--port N]        # Run daemon foreground (HTTP on port, VNP on port+1)
malt start                    # Start daemon in background
malt stop                     # Graceful shutdown via /shutdown endpoint
malt status                   # Show daemon health + session list
malt new [--name N] [--isolation <bare|restricted|capped|contained>]
         [--isolation-policy <required|preferred|disabled>]
                               # Non-bare defaults to required: unavailable isolation refuses;
                               # preferred is the explicit visible downgrade policy
malt isolation capabilities    # Report session-spawn-path tier availability and its evidence basis
malt list                     # List sessions
malt attach [ID]              # Open TUI connected to session (VNP + HTTP fallback)
malt exec ID "command"        # Run command via mash, return output (reports truncation if the
                               # reply exceeds the 1 MiB cap)
malt output ID                # Print session's current output as plain text
malt history ID               # List the session's command execution history
malt watch ID [--output]      # Stream the session's lifecycle events live (SSE); --output
                               # streams the command's raw stdout/stderr chunks instead
malt send ID "input"          # Send raw bytes to whatever is reading (NOT a command)
malt eof ID                   # Signal end-of-input to the current reader (Ctrl-D)
malt kill ID                  # Destroy session
```

The Gateway now enforces real auth (2026-07-25) — every HTTP route requires
a bearer token. `malt-bin` and `malt-mcp` read it automatically from
`~/.config/malt/api-token` (the same file the daemon's `TokenStore` writes on
first start); nothing to configure for normal CLI/agent use on the same
machine. See `docs/BACKLOG.md`'s Gateway-auth entry for what does and
doesn't have a token mechanism yet.

---

## Implementer Notes

### Code standards

- `thiserror` for library errors; `anyhow` in binary crates only.
- `tracing` for all logging. No `eprintln!`/`println!` in library crates.
- `#[non_exhaustive]` on public enums that will grow.
- `#[derive(Debug, Clone, PartialEq)]` on data types.
- Explicit re-exports only — no `pub use foo::*`.
- Rust edition 2021.
- Key deps: `vte` 0.15, `axum` 0.8, `ratatui` 0.30, `crossterm` 0.29, `clap` 4, `reqwest` 0.13, `wasmtime` 29, `wgpu` 24, `winit` 0.30, `windows-sys` 0.61.

### Type notes

- FrameElement/RenderCommand **union variants** have NO `_unknown` field
- Message structs (RenderBatch, InitialState, etc.) DO have `_unknown: Vec<u8>`
- `style` in FrameElement::Text/Paragraph is `Box<ResolvedStyle>` — `ResolvedStyle` has a `token_name: Option<String>` field (added after the initial schema; six test fixtures missed it and broke the build until fixed 2026-07-24 — if you hand-construct a `ResolvedStyle` in a test, don't forget it)
- `direction` in FrameElement::Split is `Box<Direction>`
- `child` fields are `Box<FrameElement>`, `fallback` is `Option<Box<FrameElement>>`
- `rgb` type = `(u8, u8, u8)` tuple in generated code
- `PaneId(u32)`, `SessionId(u32)` — NOT Copy, use `.clone()`
- `encode_message`/`encode_envelope` return `Result` (not infallible)
- `ToolFn` is `fn(&[String], &mut dyn Read)` — a *reader*, not a pre-read
  `&[u8]` (changed 2026-07-25). Tools that consume to the end call
  `malt_tools::read_all`; tools that stop early (`head -n`) must read
  incrementally, or they wait for an end a live session never reaches. There
  are three tool-dispatch sites in `executor.rs` and all call `tool_stdin` —
  keep it that way; guarding them individually is how two were once missed.
- mash `Env` created per session thread via `Env::from_os()` + `set_interactive(true)`
- mash `execute_list` is synchronous — perfect for session executor's thread model
- `Env` carries one `IsolationContext`; its shared established state holds the
  Windows Job Object (or, when implemented, HCS container identity). Clones
  share that carrier with coordinator status, so reports cannot name a
  different mechanism from the MASH spawn path.

---

## Where the Project Is Going

The phased feature roadmap that used to live here (Phase A–I) is retired
as of 2026-07-24 — see `docs/adr/ADR-0003-correctness-first-strategic-pivot.md`
for the full reasoning. Phase A and Phase B1 are historical fact and stay
recorded below; Phase B2 onward is replaced by a flat, priority-ordered
correctness/hardening list, informed by a five-agent audit
(`docs/findings/2026-07-24-audit-*.md`) run specifically to find what's
missing, buggy, or on shaky ground — not to plan new features.

**Guiding lens, not a task:** a 9-step demo — an agent starts `cargo test`
in a persistent session, MALT reports structured progress, a human attaches
to the same session, both see the same authoritative state, the command
requests input or fails, temporary input authority changes hands, the
session survives client disconnection, the daemon restarts and restores
session history, and the agent resumes from the last execution event
instead of scraping the terminal — is used only to judge whether a given
gap actually matters for genuine human/agent coexistence. It is not a
feature to build toward directly; no work item should be justified solely
by "the demo needs it."

**Priority order** (see `docs/BACKLOG.md` for the concrete, evidence-based
items behind each — this list is the ordering, not the detail):

- **0a. Gateway auth actually enforced.** `TokenStore`/`AuthContext`/
  `RateLimiter` are fully built and tested in isolation but never wired
  into `build_router` — every route is Admin-equivalent open to anyone who
  can reach the port today. Audit-discovered prerequisite, not on the
  original list, sequenced first because it gates safely exposing the
  Gateway to any agent at all.
- **0b. Decouple command execution from the session's single
  command-dispatch thread.** One thread, one blocking `mpsc::Receiver` per
  session; a long-running command blocks attach/output/input entirely for
  its whole duration. Audit-discovered prerequisite for genuine raw input,
  for gateway-driven execution to ever notify an attached human, and for
  attach to degrade gracefully instead of hard-timing-out.
- 1. Correct plain stdout and stderr
- 2. Real exit codes and execution IDs
- 3. Command lifecycle events
- 4. Persistent execution history
- 5. Genuine raw input
- 6. Human and agent coexistence
- 7. Fail-closed requested isolation (`required`/`preferred`/`disabled`
  policy, plus the underlying per-tier enforcement the audit found
  missing on every platform) — **re-sequenced to the front 2026-07-26 by
  ADR-0005**, which supersedes this ordering on this one point. Spec 007
  delivered the policy layer and the Windows Job Object tiers; `Contained`
  needs a privileged helper and image layers, which is the container
  substrate work
- 8. A correct TUI rendering path
- 9. Session restoration
- 10. One excellent agent client or Gateway SDK

**Explicitly paused**, with equal weight to the list above — a deliberate
decision per ADR-0003, not silent deferral: the GPU client (`maltty`)
beyond basic usability, plugin marketplace infrastructure, broad plugin
lifecycle features, remote shared deployments, exotic isolation tiers
beyond what's already built, large collections of new FrameElement/UI
variants, MCP-specific expansion, elaborate observability systems, and
maintaining multiple competing client experiences. `malt-tui` (VNP mode)
is the single reference human client; `maltty` and `malt-web` are frozen
as-is.

---

## Known Issues (evidence-based)

- **Process substitution (`<(...)`, `>()`) unimplemented.** Lexer tokenizes these as `Word`, executor has no support. No executor code exists for process substitution.
- **`{`/`}` brace tokenization context sensitivity.** Current `is_word_break` treats `{` and `}` contextually, which may need further analysis for edge cases.
- **Terminal grid rendering "staircase" bug** and **backgrounded commands not surviving through `/exec`** — see `docs/BACKLOG.md` P0/P1, both found 2026-07-24 by actually running the daemon, not by reading code.
