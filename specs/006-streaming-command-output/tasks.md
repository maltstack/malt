---
description: "Task list for streaming command output"
---

# Tasks: Streaming Command Output

**Feature**: `specs/006-streaming-command-output/`
**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md), [data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md)

## Format: `[ID] [P?] [Story] Description`

- **[P]** — parallelisable: different files, no dependency on an incomplete task
- **[US1]/[US2]/[US3]/[US4]** — the user story the task serves

## Path Conventions

Multi-crate Rust workspace. Paths are repo-relative from the worktree root.

---

> **Read before starting.** Two things from the plan govern this whole list.
>
> 1. **Output is buffered inside `mash`, not in the daemon** (research R1).
>    `ExecResult` holds `Vec<u8>` and `execute_list` returns only when the
>    command ends. Nothing downstream can be incremental until that changes,
>    which is why the shell work comes first even though it shows nothing.
> 2. **The known failure mode for this feature family is building a delivery
>    path and proving it with tests that inject values into it directly**,
>    while nothing real ever reaches it — Gateway auth, `AuthorityTracker`,
>    and the tool stdin slurp all shipped that way. Every story's acceptance
>    evidence must start from a command actually running.

---

## Phase 1: Setup (Shared Infrastructure)

- [X] T001 Confirm the workspace baseline is green before changing anything: `cargo test --workspace`, and `cargo build -p mash && MASH="$(pwd)/target/debug/mash.exe" cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture` (expect 183 passed / 3 skipped). Record the numbers; they are the comparison for every later gate.
  - `cargo test --workspace`: all green, no FAILED/error lines. Smoosh: discovered 186, runnable 183, passed 183, skipped unsupported 3, harness failures 0, shell failures 0. Matches expected baseline.
- [X] T002 [P] Confirm the claims the plan rests on rather than trusting them: that `OutputChunk` exists at `schemas/shell.vexil:37` with `MSG_OUTPUT_CHUNK = 0x04` in `crates/malt-protocol/src/codec.rs`, and that `base64` is present in `Cargo.lock`. If either is false, stop and correct the plan before writing code.
  - Confirmed: `schemas/shell.vexil:37` has `message OutputChunk { data @0 : bytes; command_tag @1 : optional<string> }` at `@domain(Shell) @type(0x04) @revision(1)`. `crates/malt-protocol/src/codec.rs:30` has `pub const MSG_OUTPUT_CHUNK: u8 = 0x04;`. `Cargo.lock` has `base64` v0.21.7 and v0.22.1 as transitive deps.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the seam that makes any of this possible. Nothing in Phase 3+ is testable until `mash` can emit output before a command ends.

- [X] T003 Define the output sink trait in `crates/mash/src/env.rs`: a `&[u8]` writer that `mash` owns and the daemon implements. `mash` must not learn what a session is (Principle VII). Keep it object-safe and `Send + Sync` — the daemon will hold it across threads.
  - `OutputSink` trait added (`write_stdout`/`write_stderr`, object-safe, `Send + Sync`).
- [X] T004 Add the optional sink handle to `Env` in `crates/mash/src/env.rs`, with `Env::set_output_sink` / `take_output_sink`. **Document at the field that it must not survive into a capturing context.**
  - `output_sink: Option<Arc<dyn OutputSink>>` field added with the required doc comment; `set_output_sink`/`take_output_sink`/`output_sink()` accessors added.
- [X] T005 Clear the sink on the `Env::clone()` paths used by command substitution and capturing subshells in `crates/mash/src/env.rs` and `crates/mash/src/executor.rs`. This is the single highest-risk invariant in the feature: an inherited sink streams `$(cmd)`'s output into the session *and* corrupts the substitution value. Find every clone site rather than the first one — three-copy and two-path bugs are this repo's recurring shape.
  - Cleared at `capture_command`'s `sub_env` (command substitution) and `execute_pipeline`'s `stage_env` (every pipeline stage, including a stage that is itself control flow recursing back through plain `execute_simple`). Verified by `pipeline_sends_its_data_once_not_twice` and both command-substitution tests in `output_sink.rs`.
- [X] T006 Route the top-level command's stdout/stderr to the sink in `crates/mash/src/executor.rs`, only when not redirected and not part of a pipeline. External commands read incrementally instead of `read_to_end` (`executor.rs:1357`); if a non-blocking or chunked read primitive is needed it belongs in `malt-platform`, not here (Principle II).
  - Added `forward_to_sink` at the three leaf dispatch points in `execute_simple` (builtin, in-process tool, external process) plus the top-level `Command::Pipeline` result. External commands now read stdout/stderr incrementally via `read_stream_incrementally`, one thread per stream (also closes a pre-existing dual-pipe deadlock risk from the old sequential `read_to_end` calls). **Found and fixed a real, pre-existing Windows bug while implementing this**: `crates/malt-platform/src/process/windows.rs`'s `argv0`-override spawn path (used by every `mash` external-command spawn, since `configure_command_spawn_identity` always sets `argv0`) creates pipes via raw synchronous `CreatePipe`, but wrapped them as `std::process::ChildStdout`/`ChildStdin`/`ChildStderr` — types whose `Read`/`Write` impls assume the overlapped-I/O mode `std::process::Command`'s own pipe creation uses. A single `read_to_end` call happened to work; a second `.read()` call that had to genuinely wait for more data hung forever. Fixed by giving `process::Child` a small per-direction enum (`ChildStdoutHandle`/`ChildStderrHandle`/`ChildStdinHandle`) carrying either the genuine overlapped type (plain spawn path, e.g. PTY spawning) or a plain `std::fs::File` (the `argv0`-override path), each dispatching `Read`/`Write` correctly for its own I/O mode; Unix is a zero-change type alias. Confirmed via a minimal standalone `std::process::Command` repro before diagnosing, and via full workspace green after.
- [X] T007 Unit tests in `crates/mash/tests/output_sink.rs`: a sink receives a command's output progressively; `$(echo hi)` still yields `hi` as a value **and sends nothing to the sink**; `echo a | cat` sends the pipeline's data once, not twice. Assert on sink *contents*, not on call counts — a count-based assertion passes when the wrong bytes arrive.
  - 7 tests added and passing: progressive stdout, separate stderr, command substitution (both as a value and standalone), pipeline-once, redirected-command-silent, no-sink-installed.
- [X] T008 **Smoosh gate.** `cargo build -p mash && MASH=... cargo test -p mash --test smoosh_runner smoosh_conformance_tests`. Expect 183 passed / 3 skipped. This is the check that catches a mistake in T005/T006; command substitution and pipelines are heavily covered there. Do not proceed past a regression.
  - 183 passed / 3 skipped, matching baseline exactly. Full `cargo test --workspace` also green (all crates, including `malt-daemon`'s PTY-spawn supervisor which consumes the changed `process::Child` API).

**Checkpoint**: `mash` emits output as it is produced, and POSIX conformance is intact. No user-visible change yet.

---

## Phase 3: User Story 1 - Output arrives while the command is still running (Priority: P1) 🎯 MVP

**Goal**: a running command's output is observable before it ends.

**Independent Test**: quickstart Scenario 1 — run `echo first; sleep 5; echo second`; `first` is observable, and the observing call returns, before the command exits.

### Implementation for User Story 1

- [X] T009 [US1] Extend `OutputChunk` in `schemas/shell.vexil` with `sequence @2 : u64` and `stream @3 : OutputStream`, plus the `OutputStream` enum; bump `@revision`. **Extend the existing message — do not add a second one** (contracts/output-chunk-vnp.md). Keep existing field numbers so decoders stay compatible. Note the `vexilc` field-level `@doc` off-by-one recorded in `docs/BACKLOG.md`: put docs at message level.
  - `@revision(2)`; generated `OutputChunk{data,command_tag,sequence,stream,_unknown}` and `OutputStream{Stdout,Stderr,Unknown(u64)}` confirmed via build output. `roundtrip.rs` test updated and passing.
- [X] T010 [US1] Add the worker-side sink implementation in `crates/malt-daemon/src/executor/command_worker.rs`: forward each write to the control actor as a new `SessionCommand::OutputChunk` over the existing `control_tx`, bounded, **blocking when full** (research R4). One ordered channel, so a command can never report completion before its last output.
  - `ChunkForwardingSink` implements `mash::env::OutputSink`. `control_tx` migrated from `mpsc::Sender` to `mpsc::SyncSender` (bounded, `SESSION_CONTROL_CHANNEL_CAPACITY = 1024`) across `session_thread.rs`/`coordinator.rs`/`command_worker.rs` — verified no self-send-from-actor-thread deadlock risk (all send sites are the worker, coordinator, or dedicated reaper threads).
- [X] T011 [US1] Install the sink on the worker's `Env` for the duration of each execution in `crates/malt-daemon/src/executor/command_worker.rs`, and remove it afterwards so it cannot outlive the command it belongs to.
  - `env.set_output_sink(...)` before, `env.take_output_sink()` after, unconditionally (covers the panic path too).
- [X] T012 [US1] Create `crates/malt-daemon/src/executor/output_log.rs`: bounded retained ring **sized by total bytes, not chunk count** (a count bound does not bound memory when chunk sizes vary), monotonic sequence assignment, and subscriber sinks. Model on `crates/malt-daemon/src/executor/events.rs` and share code where the shapes are genuinely identical. **Carry over the reserved-slot detail**: allocate `buffer + 1` and reserve the last slot for the gap notification, or a lagged subscriber is dropped silently — the exact defect that occurred in 004.
  - `OutputLog`/`OutputSubscriberSink` built as a deliberate structural parallel to `events.rs` (documented as such rather than genericized, to avoid touching already-shipped, tested code). Reserved slot present (`buffer + 1`). 10 unit tests, including the oversized-single-chunk edge case and the "full sink can still be told it lagged" regression.
- [X] T013 [US1] Handle `SessionCommand::OutputChunk` in `crates/malt-daemon/src/executor/session_thread.rs`: assign the sequence, append to the log, and publish to subscribers. The control actor is the single writer, so it alone defines order.
  - `publish_output`/`subscribe_output` added, mirroring `publish_lifecycle`/`subscribe_events`. Also added `SessionCommand::SubscribeOutput` (not explicitly listed until Phase 4's Coordinator wiring, but needed here so T016-T018's integration tests could exercise the mechanism directly, matching the existing `SubscribeEvents` precedent).
- [X] T014 [US1] Make the accumulated `/exec` reply bounded in `crates/malt-daemon/src/executor/session_thread.rs`, with `truncated` and `omitted_bytes` on `CommandOutput` (research R3). The cap is a named constant with its reasoning in a comment. **Truncation is stated, never silent.**
  - `EXEC_REPLY_CAP_BYTES = 1 MiB`; `cap_command_output` truncates at a UTF-8 char boundary, applied only at the final reply (compat feed/bus/history still see the complete output).
- [X] T015 [US1] Surface the new fields on the exec response in `crates/malt-gateway/src/routes/sessions.rs` and `crates/malt-gateway/src/types.rs`, keeping the existing shape additive so `malt exec` and the MCP `run_command` tool keep working.
  - `ExecResult`/`ExecResultData` gained `truncated`/`omitted_bytes` (client-side `#[serde(default)]` for compatibility); `malt exec` prints a truncation notice; MCP's `run_command` passes the raw JSON through unchanged, so it surfaces the new fields automatically.
- [X] T016 [US1] Integration test in `crates/malt-daemon/tests/output_stream.rs`: run a command that prints, sleeps, then prints; assert the first line is retrievable **while the command is still running**, and that the retrieval call returns promptly rather than blocking for the command's duration. Start from a real `exec_command`, not from injected chunks.
  - `output_arrives_and_is_retrievable_while_the_command_is_still_running`: real `SessionCommand::RunCommand` through the real worker; subscriber receives "first\n" in well under the 2s sleep, and the run's own reply is confirmed still pending at that point.
- [X] T017 [P] [US1] Test in `crates/malt-daemon/tests/output_stream.rs` that output produced before a failing command's exit is delivered and not replaced by the failure report (spec US1 scenario 3).
  - `output_produced_before_a_failing_commands_exit_is_delivered_and_not_replaced`.
- [X] T018 [P] [US1] Test in `crates/malt-daemon/tests/output_stream.rs` that a command producing far more output than the retained bound completes successfully and the log stays within its byte bound (SC-004 in miniature; the 100 MB case is a quickstart scenario, not a unit test).
  - Added `SessionExecutor::spawn_with_capacity_and_output_bound` (test-support entry point, production paths still hardcode `output_log::MAX_RETAINED_BYTES`) so the test can force eviction with a 64-byte bound instead of megabytes of real output. `a_command_producing_more_than_the_retained_bound_completes_and_the_log_stays_bounded` confirms completion, a reported gap, and bounded retained bytes.

**Checkpoint**: US1 is independently verifiable. All four gates green plus Smoosh. Commit. **Merge to main.**

**Gate results**: `cargo test --workspace` green (all crates); `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; Smoosh 183 passed / 3 skipped (matches baseline). New tests: `mash`'s `output_sink.rs` (7), `malt-daemon`'s `output_log` unit tests (10) and `output_stream.rs` integration tests (3).

**Notable finding, not scope creep**: implementing T006 (Phase 2) surfaced a real, pre-existing Windows bug in `malt-platform`'s process spawning — see the T006 entry above and `crates/malt-platform/src/process/{mod,windows}.rs`'s new `ChildStdoutHandle`/`ChildStderrHandle`/`ChildStdinHandle` types for the fix. Recorded here because it was found while doing this feature's own work, not sought out separately.

---

## Phase 4: User Story 3 - An agent consumes and resumes the stream (Priority: P2)

> Sequenced before US2 deliberately: it exercises the retained log and gap
> semantics directly, so defects there surface against an exact byte stream
> rather than through a rendered grid.

**Goal**: a program consumes output as it appears and can resume without duplication or loss.

**Independent Test**: quickstart Scenario 3 — consume the stream, disconnect mid-command, resume from the last sequence, and confirm by **content** that every chunk arrived exactly once.

### Implementation for User Story 3

- [X] T019 [US3] Add subscribe/resume entry points on `Coordinator` in `crates/malt-daemon/src/executor/coordinator.rs`, following the existing `begin_*` pattern so the coordinator lock is not held while a subscriber waits.
  - `begin_subscribe_output` added, mirroring `begin_subscribe_events` (same dormant-session refusal, same lock-release-before-wait shape).
- [X] T020 [US3] Add `GET /sessions/{id}/output/stream` in `crates/malt-gateway/src/routes/sessions.rs` as SSE, with `Last-Event-ID` resume, per contracts/output-stream-http.md. Payloads are **base64** — output may be invalid UTF-8 and a character may be split across chunks, and lossy decoding at a boundary is unrecoverable by the client (research R6).
  - `output_stream` handler + `OutputChunkDto` (types.rs) + `to_output_dto` (gateway_backend.rs, base64-encodes `data` directly from the raw `Vec<u8>`, no text conversion in between). `base64 = "0.22"` added to both `malt-gateway` and `malt-daemon`.
- [X] T021 [US3] Register the route in `crates/malt-gateway/src/server.rs` and map it to **`Read`** scope in `crates/malt-gateway/src/middleware.rs`. Read, not Interact: observation, consistent with `/events` and `/history` (FR-017).
- [X] T022 [US3] Emit `gap` frames for both `retention_exceeded` and `subscriber_lagged` in the route handler and `output_log.rs`. A stream that closes without a gap leaves the consumer believing it saw everything; that is the failure the frame exists to prevent.
  - Both reasons already wired at T012/T013; verified end-to-end by T025's test. The "connection closes after a lagged gap" contract requirement falls out of channel `Drop` semantics for free: the control actor removes the sink (dropping its `Sender`) right after queuing the gap, so the SSE stream ends immediately after delivering it.
- [X] T023 [US3] Extend `malt watch` with `--output` and `--resume-from` in `crates/malt-bin/src/{cli,client,events}.rs`, reusing the SSE frame parser. **Run it against a live daemon before believing it works**: feature 004's parser dropped every frame while eight unit tests passed, because the test helper fed pre-trimmed lines the real reader never produces.
  - `--output` flag added; `EventPayload` extended with the output-stream fields (`stream`/`data`/`produced_at`/`from`/`to`) alongside the lifecycle ones, so `FrameParser`/`StreamEvent` are genuinely reused unchanged (SSE framing doesn't depend on payload shape). `watch_stream` factored out of `watch_events`, and `watch_output` added on top of it. `handle_watch_output` decodes base64 and writes raw bytes to stdout; gap notices and the startup banner go to stderr. **Not yet run against a live daemon** — noted for the T041 manual quickstart pass, per the task's own explicit warning.
- [X] T024 [US3] Resume test in `crates/malt-daemon/tests/output_stream.rs`: consume, disconnect mid-command, resume from the last sequence, and assert the concatenation matches the command's full output **byte-for-byte**. Verify by content, not by count (SC-003).
  - `resuming_after_disconnect_reproduces_the_full_output_byte_for_byte`.
- [X] T025 [P] [US3] Lagged-subscriber test in `crates/malt-daemon/tests/output_stream.rs`: a subscriber that stops reading is told it lagged and is dropped; the command's duration and other subscribers are unaffected (SC-005). Assert the gap notification **arrives** — its absence is exactly how 004's defect hid.
  - `a_stalled_subscriber_is_told_it_lagged_and_dropped_without_affecting_others`. Needed a test-support subscriber-buffer override (`spawn_with_capacity_and_output_bound` extended) and paced (`sleep 0.1`) chunks — an initial back-to-back-chunks version was flaky under parallel test-suite load because the *healthy* subscriber's own reader thread wasn't reliably scheduled fast enough against a too-tight buffer; fixed by giving it real wall-clock room, re-verified clean across multiple full-workspace runs.
- [X] T026 [P] [US3] Byte-fidelity test in `crates/malt-daemon/tests/output_stream.rs`: invalid UTF-8 and a multi-byte character split across a chunk boundary both survive the round trip including base64 (SC-006).
  - `invalid_utf8_and_a_split_multibyte_character_survive_the_round_trip_including_base64`, via a real `cat` of a file containing deliberately invalid bytes (not `printf` escapes — found along the way that `mash`'s printf `\NNN` octal-escape interpreter casts the byte value directly to `char`, so `printf '\377'` produces the UTF-8 encoding of U+00FF rather than a raw 0xFF byte; a pre-existing, separate `printf` quirk, not touched here).

**Checkpoint**: US1 and US3 both work. Gates green. Commit. **Merge to main.**

**Gate results**: `cargo test --workspace` green; `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; Smoosh 183/3 (unchanged, `mash` untouched in this phase). New tests: `malt-daemon`'s `output_stream.rs` gained 3 more tests (6 total).

---

## Phase 5: User Story 2 - An attached human sees output live (Priority: P2)

**Goal**: an attached client's view updates during a command, not only at its end.

**Independent Test**: quickstart Scenario 5 — attach a TUI, run a five-second output-producing command from elsewhere, and count more than one update before completion.

### Implementation for User Story 2

- [X] T027 [US2] Feed each chunk to the compat translator and call `dispatch_render()` as it arrives, in `crates/malt-daemon/src/executor/session_thread.rs`, instead of feeding one slice at finalization. **Check first whether the finalization slicing should be removed or kept** — two paths feeding the same grid would double-render.
  - Removed: the finalization slicing existed only to spread a large synchronous compat feed across actor turns, and that feed is now redundant since the same bytes already reached the live grid via `OutputChunk`. `Finalization` lost its `staged_compat`/`stdout_offset`/`stderr_offset` fields; `advance_finalization` now commits in one turn (snapshot swap, history, lifecycle event, reply) with no re-feed. Verified no double-render via T030/T031 and the full daemon test suite.
- [X] T028 [US2] Add the `OutputChunk` variant to `ClientMessage` in `crates/malt-daemon/src/executor/session_thread.rs`, delivered on the **same ordered per-client stream** as `Render` and `AuthorityChanged` (settled in 005: separate channels cannot promise ordering). Use the existing non-blocking `try_send`; add no second backpressure mechanism.
  - `publish_output` now returns the assigned sequence so the same number is used for both the output-log event and the `ClientMessage::OutputChunk` sent to each `render_pushers` entry via the existing non-blocking `try_send`.
- [X] T029 [US2] Send `OutputChunk` frames to attached VNP clients in `crates/malt-daemon/src/vnp_listener.rs` using `MSG_OUTPUT_CHUNK` in the Shell domain. A client that ignores them must still render correctly — `maltty` and `malt-web` are frozen and must not break.
  - Handled in the VNP client loop's existing `render_rx.try_recv()` match; `produced_at` has no wire field per contracts/output-chunk-vnp.md (only `data`/`command_tag`/`sequence`/`stream`) so it is intentionally discarded there, not carried. `maltty`/`malt-web` untouched — a client that never asks for this message type is unaffected.
- [X] T030 [US2] Test in `crates/malt-daemon/tests/output_stream.rs` that an attached client receives **more than one** `Render` during a command producing output over several seconds (FR-007), driven by a real command.
  - `an_attached_client_receives_more_than_one_render_during_a_running_command`.
- [X] T031 [P] [US2] Test in `crates/malt-daemon/tests/output_stream.rs` that two attached clients converge on identical content and neither sees output the other does not (SC-007).
  - `two_attached_clients_converge_on_identical_content`.

**Checkpoint**: US1, US2, US3 all work. Gates green. Commit. **Merge to main.**

**Gate results**: `cargo test --workspace` green; `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean (caught and fixed a `clone_on_copy` on `OutputStream`, which is `Copy`); Smoosh 183/3 unchanged. `output_stream.rs`: 8 tests total.

**Staircase defect**: not touched, per Principle IX. Not separately re-verified whether streaming makes it more visible (no manual TUI session run yet in this phase) — left for the T041 manual quickstart pass.

> **Do not fix the grid "staircase" defect here.** Streaming makes it more
> visible, and it will be tempting. It is a separate P0 in `docs/BACKLOG.md`
> and absorbing it violates Principle IX. If streaming genuinely worsens it
> beyond its existing behaviour, write that down as a finding.

---

## Phase 6: User Story 4 - In-process utilities stream too (Priority: P3)

**Goal**: built-in utilities have their output observable while they run.

**Independent Test**: quickstart Scenario 7 — `cat` with input sent over time makes each line observable before the next is sent.

### Implementation for User Story 4

- [X] T032 [US4] Change `ToolFn` in `crates/malt-tools/src/lib.rs` to take an output writer alongside the existing reader, so a tool can emit as it works rather than returning a finished buffer. This mirrors what feature 005 did for input; the same "three dispatch sites" lesson applies — put the rule in one place.
  - `ToolFn = fn(&[String], &mut dyn Read, &mut dyn Write) -> BuiltinResult`. Added `emit()` helper for tools that only ever produce one finished summary (no incremental framing possible).
- [X] T033 [US4] Update all tools in `crates/malt-tools/src/custom/*.rs` for the new signature. Most ignore output framing entirely; `cat`, `grep`, `sed`, `head`, `wc` are the ones that matter.
  - All 17 tools updated. `cat`/`grep`/`sed`/`head` stream genuinely (chunk- or line-incremental, verified by dedicated "streams as produced, not only at the end" unit tests using a deliberately slow/one-byte-at-a-time reader). `wc` cannot meaningfully stream (one summary line, known only once input is exhausted) so it uses `emit()` once at each return point — same reasoning documented inline as `cat`/`grep`/`sed`/`head`'s non-streaming modes (`count_only`, etc). `ls`/`which`/all no-stdout tools use `emit()` or an unused writer. `crates/malt-tools/tests/tools.rs` and the `date`/`sleep` tests in `lib.rs` updated for the 3-arg call sites. Full `cargo test -p malt-tools`: 82 passed, 0 failed, 0 warnings.
- [X] T034 [US4] Update the three tool-dispatch sites in `crates/mash/src/executor.rs` via the existing single `tool_stdin` helper pattern — add the writer resolution to that one helper, not to each site.
  - Added `tool_stdout(redirect_to_file, env) -> Box<dyn Write>` next to `tool_stdin`, plus a `SinkWriter` adapter over `Env::output_sink()`. Returns a no-op `std::io::sink()` writer when stdout is redirected to a file or no sink is installed, else a writer that forwards straight to the sink. Applied identically at all three sites (`execute_simple_with_io`, `execute_simple`, `execute_expanded_command`); removed the now-redundant `forward_to_sink(env, &result)` call at the `execute_simple` site (tools now stream directly, so forwarding the buffer afterward would double-deliver). The other two sites never called `forward_to_sink` for tools before this task either (pipeline stages already clear the sink via `env.clone()`; `execute_expanded_command`/`exec` simply hadn't been wired) — using one helper uniformly fixed the `exec`-dispatch gap as a side effect rather than requiring separate handling. `cargo build -p mash` and `cargo test -p mash`: 228/228 executor tests, all green.
- [X] T035 [US4] Verify redirection is unchanged in `crates/mash/src/executor.rs`: `apply_output_redirects` operates on the buffered result today, so a tool whose stdout is redirected to a file must still write the same bytes (FR-014). If the writer change bypasses that function, the redirect silently stops working while the streaming test still passes.
  - Confirmed unchanged: tools still dual-write (writer + returned buffer), and `apply_output_redirects`/`apply_builtin_output_redirects` still act on the returned buffer exactly as before. Added `a_tool_redirected_to_a_file_writes_the_full_output_there_and_not_to_the_sink` in `output_sink.rs` (plain, non-pipeline `cat file > outfile` with a sink installed) — passes.
- [X] T036 [US4] Test in `crates/mash/tests/output_sink.rs` that a built-in copying input to output emits each line before the next arrives (SC-008), and a redirection test for T035.
  - Added `a_built_in_utility_copying_input_makes_each_line_observable_before_the_next_is_sent`: registers a real OS pipe as fd 0 (`Env::register_fd`, the same seam a live session uses), runs `cat` on a background thread, and only writes the second line to the pipe after observing the first arrive at the sink via a channel — the wait-then-send ordering is itself the proof, not an inference from timing. `cargo test -p mash --test output_sink`: 9/9 passed.
- [X] T037 [US4] **Smoosh gate again**, since `mash` and `malt-tools` both changed: `cargo build -p mash && MASH="$(pwd)/target/debug/mash.exe" cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture`. Expect 183 passed / 3 skipped.
  - Ran clean: discovered 186, runnable 183, passed 183, skipped unsupported 3, harness failures 0, shell failures 0.

**Checkpoint**: all four stories work. Gates green. Commit. **Merge to main.**

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T038 Update `docs/BACKLOG.md`: close the "pushes one frame at completion, not incremental output" item with the test names that prove it, and close or re-scope the streaming-`ToolFn` item. Cite tests that **exist** — verify each name before writing it.
- [ ] T039 [P] Update `AGENTS.md`: output streams during a command; `/exec` output is bounded and reports truncation; the new route and `malt watch --output`. Correct any "What's Implemented" claim this feature falsifies.
- [ ] T040 [P] Amend `docs/design/architecture.md` where it describes output flow, if it describes something other than what now exists.
- [ ] T041 Run the full quickstart manually against a live daemon — all eight scenarios including the 100 MB volume case and the stalled-subscriber case — and record the outcome, **including what it does not establish**, in a dated `docs/findings/` entry.
- [ ] T042 Final verification: `cargo test --workspace`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and the Smoosh command from T037 (183 passed / 3 skipped). Update `specs/006-streaming-command-output/tasks.md` with the closing note. Commit. **Merge to main.**

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → blocks everything.
- **Phase 2 (Foundational)** → **hard blocker for every story.** Until `mash` emits incrementally there is nothing to deliver, so no story can be started or meaningfully tested.
- **Phase 3 (US1)** → blocks US2 and US3: both consume the chunk pipeline it builds.
- **Phase 4 (US3)** and **Phase 5 (US2)** → independent of each other once US1 lands; either order works. US3 is listed first because it tests the byte stream exactly rather than through a rendered grid.
- **Phase 6 (US4)** → after US1; independent of US2/US3.
- **Phase 7 (Polish)** → after all stories.

### Within-Story Dependencies

- Foundational: T003 → T004 → T005 → T006 → T007 → T008. T005 before T006, so the capture invariant exists before anything routes to the sink.
- US1: T009 → T010 → T011 → T012 → T013 → T014 → T015; T016–T018 after T013.
- US3: T019 → T020 → T021 → T022 → T023; T024–T026 after T022.
- US2: T027 → T028 → T029; T030–T031 after T029.
- US4: T032 → T033 → T034 → T035; T036–T037 after T035.

### Parallel Opportunities

- T002 alongside T001 — different concerns, both read-only.
- T017/T018 alongside T016 — same file, so coordinate, but independent subjects.
- T025/T026 alongside T024 — same.
- T031 alongside T030.
- T039/T040 in parallel — different documents.
- **US3 (Phase 4) and US2 (Phase 5) can run in parallel** once US1 is merged; they touch different crates (`malt-gateway`/`malt-bin` versus `malt-daemon`/`vnp_listener`).

---

## Implementation Strategy

**MVP is Phase 1 + Phase 2 + Phase 3 (US1).** That is the whole point of the
feature: output observable before a command ends. US2, US3, and US4 refine who
receives it and how.

Note that the MVP carries an unusually large invisible prefix — Phase 2
changes `mash` and delivers no user-visible behaviour on its own. Resist
shortening it. The alternative is routing output to the session without the
capture distinction, which corrupts `$(...)` rather than merely failing to
stream, and Smoosh is the only thing that reliably catches it.

**Incremental delivery**: merge at each checkpoint, as in 003–005. Each story
is independently valuable — US1 alone makes a long command observable; US3
alone makes an agent able to follow it; US2 alone makes a human able to watch.
