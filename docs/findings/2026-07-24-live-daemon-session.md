# Findings: First Live Daemon Session After Revival

Date: 2026-07-24
Context: First time `malt` was actually run end-to-end (build → daemon → session →
real commands) since the project was picked back up after ~3.5 months dormant.
Everything before this point in the day's work was code reading, test running,
and the malt-stack revert — this is the first evidence about the actual product
experience, not just the code.

Method: built `malt-bin`, ran the real binary against a real daemon, drove it via
the CLI and the gateway HTTP API directly (`curl`), inspected real output. Not
simulated, not unit-tested — this is what a user hitting these code paths would
actually see.

## What's confirmed working

### Daemon persistence survives real dormancy
`malt start` brought up the daemon and it loaded two sessions from disk —
`opencode-test` (id 1) and `test-mcp` (id 2) — both pre-existing from actual
prior use, both `Dormant` but intact, no corruption, no crash. This is the
`.bak` backup / corruption-quarantine session store design (CLAUDE.md's
"Session store hardened" claim) surviving a real 3.5-month gap, not a
freshly-written test fixture.

### Command execution is real and stateful
```
malt exec 3 "pwd"                    → C:/Users/mamuk/projects/orix/malt
malt exec 3 "echo hello from malt session" → hello from malt session
malt exec 3 "cd /tmp && pwd"         → C:/tmp
malt exec 3 "pwd"  (separate call)   → C:/tmp   ← state persisted
```
The `cd` from one `exec` call was still in effect on the next, separate `exec`
call. The daemon is holding a genuinely live shell environment between
requests, not resetting state per call.

### `send` (raw input injection) works
`malt send 3 "echo interactive-input-test"` → text appeared in subsequent
`/output` correctly. This is the mechanism `malt-mcp`'s `send_input` tool
proxies to (`POST /sessions/{id}/send`) — confirmed working directly, without
needing to stand up the MCP stdio server.

### Isolation tiers are real at the API level
```
curl -X POST http://127.0.0.1:7700/sessions -d '{"name":"restricted-test","isolation":"Restricted"}'
→ {"ok":true,"data":{"id":4,...,"isolation":"Restricted","state":"Active"}}
```
Confirms today's Job Object wiring (session_thread.rs → mash's Env →
executor's spawn call sites) is reachable through the real daemon, not just
through the unit test written for it. Not re-verified via live process
inspection here — that was already proven rigorously via the unit test
(`assign_child_to_session_job_actually_terminates_process_via_job`), which is
stronger evidence than a manual `Get-Process` poll would be against a
sub-second-lived process.

## Bugs found — real, not hypothetical

### 1. Terminal grid rendering: cursor-position "staircase" bug — HIGH priority

`GET /sessions/{id}/output` returns a real character-cell grid (`StyledGrid`
type — rows of styled character cells, not plain text). Rendered as text, the
rows look like this after a few commands:

```
lock
    Cargo.toml
              VEXIL_GAPS.md
                           C:/tmp
                                 C:/tmp
                                       interactive-input-test
```

Each successive line starts further right than the last: 0, 4, 14, 27, 33, 39
spaces of leading padding. Not random corruption — a specific, growing offset,
consistent with the cursor's column position not resetting to 0 on newline
(possibly carrying over from the previous line's end-of-text column instead).

**Why this matters more than it might look:** the shell logic underneath is
completely correct — every command executed properly, output content is
accurate. But if this is what the real ratatui/GPU renderer draws from the
same `FrameElement`/`RenderCommand` pipeline, using `malt` interactively today
would look visibly broken on screen, independent of how solid the shell engine
is. This is very likely the single biggest gap between "the shell works" and
"this is usable as a daily driver."

**Not yet investigated:** which layer owns the bug — `malt-renderer`'s
`FrameWalker`/`DirtyTracker`, `malt-compat`'s VT-to-grid translation, or
`mash`'s own line/cursor tracking. Needs a focused debugging session, not
speculation.

### 2. Backgrounded commands (`&`) don't appear to survive through `exec` — MEDIUM priority, not root-caused

```
curl -X POST .../sessions/4/exec -d '{"command":"ping -n 30 127.0.0.1 &"}'
→ {"ok":true,"data":{"command_id":0,"output":"","exit_code":null}}
```
Empty output, and no `ping` process was found running afterward (checked via
`Get-Process` immediately after). By contrast, `mash`'s own executor test
suite has real, passing background-job tests (`background_pipeline`,
`wait_builtin_flushes_background_group_output`, etc.), so the shell's own `&`
handling works when mash runs standalone. This suggests the gap is specific to
how the daemon's `/exec` route handles a command whose job is backgrounded —
possibly the session's process/job bookkeeping not surviving past the HTTP
response, or the backgrounded child being cleaned up too eagerly.

**Not yet investigated:** haven't traced whether this is a daemon-side bug,
a client/timing artifact (the check ran late relative to a very short-lived
process), or something else. Foreground commands with a few seconds of
runtime were confirmed to execute correctly and return real output
(`ping -n 5 127.0.0.1` returned real ping output), so it's specifically the
backgrounding path, not general exec, that's suspect.

## Smaller gaps

- **`malt new` has no `--isolation` flag.** Only `--name` is exposed. The
  gateway API accepts `isolation` in the create-session body and today's Job
  Object wiring is real and tested, but there's currently no way for an
  actual user to reach a non-Bare tier through the primary CLI — only by
  calling the gateway HTTP API directly, as done for this investigation.
- **`exec` responses always show `"exit_code": null`.** Observed on every
  `exec` call in this session, including ones that clearly succeeded (e.g.
  `echo`). Might be an intentional async-result design (exit code fills in
  later via a different route) rather than a bug — not confirmed either way.

## What this changes

Before this session, the working assumption (from reading code, architecture
docs, and test results) was that `malt` was close to daily-driver-usable and
the main gaps were backend/isolation work. This is the first real evidence
against that assumption: the rendering bug is a plausible, concrete blocker
that no amount of backend work would surface or fix. See `docs/BACKLOG.md`
for how these findings were prioritized.
