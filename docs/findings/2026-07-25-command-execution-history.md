# Finding: command execution history against a live daemon (2026-07-25)

Manual validation of `specs/003-command-execution-history/quickstart.md`
against a real daemon on port 7800, after the implementation landed
(`13a0e27`). Recording it because two things surfaced that the test suite
does not and cannot show, and one of them made the quickstart as originally
written un-runnable.

## What was verified end-to-end

Real daemon, real mash, real `.vxb` persistence on disk — not a test fixture.

**Recording (quickstart Scenario 1) — works.** Four executions through
`malt exec`, retrieved with `malt history`:

```
    1  23:30:36         2ms     0  echo hello
    2  23:30:36         1ms     1  false
    3  23:30:36         0ms     1  echo 'unterminated
    4  23:30:36        1.0s     0  sleep 1
```

Every category is right: a success, a real non-zero exit code (not
flattened to 0), a parse error recorded as an attempted execution rather
than dropped, and a 1-second command whose measured duration is actually
1.0s — meaning `started_at` is captured before execution rather than
alongside `finished_at`. A session with nothing run reports "no command
history", not an error.

**Restart survival (Scenario 2) — works.** `malt stop` (graceful, persists),
restart the daemon, `malt history 5` returns all four entries with identical
ids, timestamps, and exit codes.

**Automation surface (Scenario 3) — works.** `GET /sessions/5/history` with
a bearer token returns the same data as JSON; no token gives 401; an unknown
session gives 404.

## Surprise 1: `exec` after restart is refused, so the quickstart's
id-monotonicity step could not run as written

Scenario 2 originally ended with "exec after restart, observe a greater
`command_id`". That cannot be done through the CLI: a restored-from-disk
session is `Dormant`, and `malt exec` on a dormant session fails with
`session SessionId(5) is dormant — attach to restore it`. Only attaching
(TUI/VNP) wakes it.

This is pre-existing behavior, not a regression from this feature, and it is
arguably correct — but it means the CLI alone cannot demonstrate
post-restore id monotonicity. The behavior *is* covered, by
`command_history_survives_dormant_restore_and_ids_stay_monotonic`
(`malt-daemon/tests/gateway_backend.rs`), which drives the restore through
`register_vnp_client` the way a real attach would. quickstart.md has been
corrected to say so rather than instructing a step that fails.

## Surprise 2 (a good one): history reads work on a dormant session

`GET /sessions/5/history` against the `Dormant` session returned the full
history without restoring it — the design intent from research R5 ("listing
what a session already ran should not restore it as a side effect"),
confirmed working against a real on-disk snapshot rather than only in the
unit test. `malt list` still showed the session as `Dormant` afterwards.

This is the one place where history behaves *better* than the adjacent
output/exec routes, which either error or force a wake.

## Incidental observation: inconsistent not-found handling across routes

Worth writing down because it is a real inconsistency this work happened to
expose rather than introduce:

- `malt history 1` (unknown session) → `session not found: 1` (a clean 404)
- `malt exec 1 "..."` (unknown session) → `internal error: session not
  found: SessionId(1)` (a 500-shaped internal error)

`exec_command` in `gateway_backend.rs` maps every `DaemonError` through
`GatewayError::Internal`, so a genuinely-absent session is reported as an
internal failure. `get_command_history` resolves the pane first specifically
so an unknown session is a real `SessionNotFound`. `get_output_text` has the
same flaw as `exec`. Not fixed here — out of scope for this feature, and
changing exec's error mapping deserves its own change with its own tests.
Added to `docs/BACKLOG.md`.

## Cleanup

Test session destroyed, daemon stopped. `malt kill 5` on a dormant session
reported `destroy session response contained no data` while still removing
the session — a separate, pre-existing response-shape wart in the destroy
route, unrelated to this feature. Not investigated further.

## Addendum (post-002 rebase, same day)

Re-validated after rebasing onto feature 002 (responsive session control),
which moved MASH execution to a dedicated worker thread. The behavior this
feature could only assert structurally before is now directly observable
against a live daemon.

While a 4-second command runs, `malt history` at t+1s returns in **113 ms**:

```
    1  00:10:24  incomplete     -  sleep 4; echo slow
```

and the same entry after completion:

```
    1  00:10:24        4.0s     0  sleep 4; echo slow
```

Over raw HTTP during a 5-second command, `finished_at` and `exit_code` are
both `null` as specified, returned in 117 ms. The in-flight record is
finalized in place rather than duplicated.

**One false alarm worth recording so it is not re-investigated.** The first
CLI measurement of this appeared to take 1.99 s and showed the command
already complete, which looked like history blocking behind execution. It
was cold binary start: the same command warm takes 43 ms, and `malt --help`
(which touches no daemon at all) took 45 ms warm versus the same ~2 s cold.
The daemon was never the bottleneck — confirmed by the curl measurement
above, which bypasses the CLI entirely.
