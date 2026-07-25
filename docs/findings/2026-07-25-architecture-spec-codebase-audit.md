# Architecture, Specification, and Codebase Audit - 2026-07-25

## Scope and method

This audit compares the checked-out Rust implementation with the target architecture, ADRs, backlog, schemas, and feature Specifications 001 through 004. It records verified evidence, not release readiness. The existing untracked vision-execution-uris.md was read but not edited.

| Check | Result |
| --- | --- |
| cargo build --workspace | Passes with warnings. |
| cargo test --workspace | Passes. |
| cargo fmt --check | Fails across multiple tracked Rust files. |
| cargo clippy --workspace --all-targets -- -D warnings | Fails first at crates/malt-term/src/history.rs:136. |

Tracked means the backlog acknowledges a gap. It does not make an incorrect success response safe.

## Critical findings

### A-01 - VNP is an unauthenticated, authority-free control plane

Severity: Critical security. Status: authority gap tracked; transport authentication gap not adequately tracked.

The raw loopback TCP listener accepts every client, starts a thread, and sends the session inventory before authentication (crates/malt-daemon/src/vnp_listener.rs:56-70,111-124). It accepts caller-supplied session IDs for attachment and routes keys by session only (:146-176,379-397). Registration does not attach the client to AuthorityTracker (executor/coordinator.rs:459-522), and KeyInput carries no client ID or authority check (executor/session_thread.rs:663-686).

Any local process that can connect can enumerate, observe, resize, and inject input into sessions. This violates the multi-client authority contract (docs/design/architecture.md:109-128) and the local peer-credential authentication model (:1169-1199). HTTP bearer authentication does not protect VNP.

Correction: use authenticated local transport or authenticated VNP handshake, bind it to a principal, and enforce attach, detach, claim, and client identity on every input operation.

### A-02 - Requested isolation can succeed without requested isolation

Severity: Critical security. Status: tracked, but Spec 001 is not honestly closed.

Isolation setup returns unit and logs Job Object creation failure while continuing uncontained (executor/session_thread.rs:67-118). Capped and Contained map to the same placeholder Job Object limits (:42-54); the code states Contained does not launch inside HCS (:80-90). Non-Windows has no enforcement path. Gateway creation reports the requested enum rather than verified enforcement (crates/malt-daemon/src/gateway_backend.rs:147-164).

This conflicts with Spec 001, which requires failure when the requested tier is unavailable, rejected, or not honoured (specs/001-cli-isolation-flag/spec.md:53-73,80-88). The backlog confirms the absent required/preferred policy (docs/BACKLOG.md:545-566).

Correction: make capability probing and setup return a real enforcement outcome before creation succeeds; add policy and effective-capability reporting; reject unavailable required tiers; never label Job Object caps as Contained/HCS.

## High findings

### A-03 - Gateway credentials are predictable, leaked, and may be non-durable

Severity: High security and availability. Status: new.

generate_random_token derives both fields from epoch nanoseconds and fixed arithmetic, not a CSPRNG (crates/malt-gateway/src/auth.rs:121-131). Directory creation and token-file writes are ignored (:95-111), and the daemon prints the admin token to stdout (crates/malt-bin/src/daemon.rs:36-38).

Correction: use OS CSPRNG bytes, atomically write an owner-only token or fail startup, and never print the secret.

### A-04 - Auth wiring breaks the clean-machine default CLI flow

Severity: High availability regression. Status: new.

MaltClient reads a token once when constructed (crates/malt-bin/src/client.rs:127-138), but main constructs it before starting the daemon (main.rs:13-18,41-58). On a clean machine the daemon then creates the token while the stale client sends unauthenticated health probes and default mode fails. Existing token files mask this.

MaltClient::shutdown ignores HTTP status and body (client.rs:162-166), so malt stop can report success after 401.

Correction: reload/create the client after daemon startup and treat non-success HTTP results as errors.

### A-05 - Rate limiting is a permanent lifetime counter

Severity: High availability; medium security. Status: new.

RateLimiter retains only HashMap token-to-count; check increments forever and no production path refills it (crates/malt-gateway/src/rate_limit.rs:6-42; crates/malt-bin/src/daemon.rs:14-17,36-39). A valid token reaches permanent 429 after 100 lifetime requests. It also lacks the architecture's per-session/global limits, rate metadata, and payload-size limits (docs/design/architecture.md:1149-1163).

Correction: implement a monotonic-clock bucket/window with eviction, metadata, and tested identity/session/global dimensions; bound bodies before handlers allocate.

### A-06 - ProcessSupervisor kill removes bookkeeping only

Severity: High correctness and resource security. Status: new.

ProcessSupervisor::kill only removes the map entry (crates/malt-daemon/src/supervisor/mod.rs:72-78). Dropping its Child does not kill it: Unix only nonblocking-reaps and Windows closes the handle (crates/malt-platform/src/process/mod.rs:307-382). Restored Compat panes create supervisor-owned processes (executor/coordinator.rs:772-825), while session destruction does not kill them.

Correction: terminate and wait for process or isolation group before removal; invoke it at destroy/shutdown and test a real PID termination.

### A-07 - Send executes shell commands instead of writing raw input

Severity: High contract and correctness. Status: tracked.

Gateway send_input submits text as new execution and waits up to 30 seconds (crates/malt-daemon/src/gateway_backend.rs:210-223). WriteInput UTF-8-decodes, trims, and executes a top-level command (executor/session_thread.rs:606-612). It cannot deliver control bytes, passwords, or input to a running process, and can execute an unintended command. This conflicts with direct focused-pane input in docs/design/architecture.md:88-96.

Correction: reserve exec for execution; implement byte-preserving authority-aware stdin/PTY input and test a process waiting for input.

### A-08 - VNP permits pre-handshake thread exhaustion

Severity: High local denial of service. Status: new.

The accept loop creates an unbounded OS thread per connection (crates/malt-daemon/src/vnp_listener.rs:56-70) and sets its first read timeout only after blocking handshake work (:135-140). Connect-and-stall clients retain threads and sockets indefinitely.

Correction: apply a bounded handshake deadline before reading and cap connection handlers.

## Medium findings

### A-09 - Pane endpoints return successful no-ops

Severity: Medium API correctness. Status: tracked.

split_pane ignores input and returns fabricated ID 0 (crates/malt-daemon/src/gateway_backend.rs:346-359); close_pane always returns Ok ( :361-363). The architecture advertises pane management (docs/design/architecture.md:1122-1128).

Correction: return typed unavailable or 501 until real daemon-owned layout mutations exist.

### A-10 - Gateway response data is not authoritative

Severity: Medium API correctness. Status: new.

The coordinator auto-suffixes duplicate names (crates/malt-daemon/src/executor/coordinator.rs:148-172,192-197), but the create response echoes the request name (gateway_backend.rs:147-164). A response can say foo while later list reports foo-2.

Real deletion of an absent ID returns success because the backend returns Ok and the coordinator silently no-ops (gateway_backend.rs:183-187; executor/coordinator.rs:351-377). Route tests use a different mock, masking this.

Correction: return stored coordinator data and typed NotFound errors; cover the real backend contract.

### A-11 - Compat restore selects the wrong pane and drops isolation

Severity: Medium correctness and security. Status: partly tracked.

Restore selects the first BTreeMap pane rather than persisted focus (crates/malt-daemon/src/executor/coordinator.rs:737-742). Its comment says Compat restore is unimplemented (:718-722), although it launches Compat. The code explicitly says its restored process is outside the session Job Object (:813-819).

Correction: restore persisted focus, correct the comment, and route Compat spawn through verified isolation.

### A-12 - Elevation helper reports success without privileged work

Severity: Medium security and API deception. Status: tracked.

Except symlink creation, every elevation operation calls stub_success and returns Ok empty bytes (crates/malt-elevate/src/dispatch.rs:39-65).

Correction: return typed Unsupported or NotImplemented until each operation has implementation and authorization.

### A-13 - Hard invariants have production violations

Severity: Medium reliability and governance. Status: new.

The constitution bans production unwrap/expect and OS calls outside malt-platform, but DebouncedStore panics on flush-thread spawn (crates/malt-daemon/src/store/debounce.rs:33-46), nine SharedFdRegistry methods panic after mutex poison (crates/malt-platform/src/vfs/fd.rs:467-507), and Io::File::clone panics on descriptor exhaustion (crates/malt-platform/src/process/mod.rs:71-89). The daemon supervisor directly uses std::os::windows::io (crates/malt-daemon/src/supervisor/mod.rs:142-147); malt-tools contains direct OS FD/symlink calls (crates/malt-tools/src/custom/fds.rs:21-56 and custom/ln.rs:117-120).

Correction: propagate structured errors, deliberately recover from poison where appropriate, and move OS-specific work behind malt-platform.

### A-14 - FrameWriter lacks the reader's size bound

Severity: Medium protocol robustness. Status: new.

FrameReader rejects payloads exceeding PROTOCOL_MAX_FRAME_SIZE (crates/malt-protocol/src/framing.rs:96-121), while FrameWriter casts payload.len to u32 without the same check (:149-154). It can emit a peer-rejected frame or truncate an oversized advertised length.

Correction: validate before narrowing, return FrameTooLarge, and test both boundaries.

### A-15 - Vi dd state leaks across sessions

Severity: Medium multi-session correctness. Status: new.

The first d is stored in a process-global AtomicBool (crates/malt-term/src/keymap.rs:235-252,321-324), not in Editor. A d in one session can make d in another clear that second editor line as dd.

Correction: make pending operator state per Editor and add an interleaved two-editor test.

### A-16 - User-facing code retains active stubs and false capability paths

Severity: Medium product completeness. Status: deliberately deferred in part.

maltty uses startup expects and ignores render lines/clears the frame (crates/maltty/src/renderer.rs:17-45,77-125). ThemeResolver is a no-op/default-colour stub (crates/malt-renderer/src/theme.rs:6-34). malt-term ignores history navigation, completion, ex mode, and search (crates/malt-term/src/keymap.rs:143-149,374-377). MASH still stubs select, coproc, umask, and limited ulimit behaviour (crates/mash/src/executor.rs:615-645,4163-4200).

Correction: implement, hide behind an unavailable capability, or document unsupported. Do not return apparent success.

### A-17 - Formatting and lint gates are red

Severity: Medium delivery hygiene. Status: formatter drift known; clippy evidence new.

The complete formatter check fails over many tracked files. Clippy with warnings denied fails at malt-term/src/history.rs:136. Completed feature tasks therefore do not establish current quality-gate readiness.

Correction: make formatting and adopted clippy policy deterministic CI gates; format in a dedicated mechanical commit and resolve or explicitly scope lint policy.

## Documentation and specification findings

### A-18 - Backlog priority header contradicts completed-work record

Severity: Medium workflow documentation. Status: new.

The priority header says Gateway auth, responsive control, execution history, and restoration are pending (docs/BACKLOG.md:20-36), while the same document records them fixed/delivered (:40-91,310-376,481-518). This can route work back into completed features.

Correction: replace the header with current unresolved priorities; keep completed work in historical/Done sections.

### A-19 - Feature statuses make delivered work appear Draft

Severity: Medium workflow documentation. Status: new.

Specs 002 through 004 still say Status: Draft (specs/002-responsive-session-control/spec.md:7, specs/003-command-execution-history/spec.md:7, specs/004-command-lifecycle-events/spec.md:7), although their task lists are fully checked and the backlog records delivery. Spec 001 tasks are checked but A-02 shows its fail-closed requirement is unmet.

Correction: mark only fulfilled specs Implemented or Closed; re-open or revise Spec 001 until its contract is honest.

### A-20 - Architecture, schema, ADR, and quickstart status are stale

Severity: Medium design governance and documentation. Status: new.

The architecture says Gateway-to-daemon communication is exclusively Bus-based (docs/design/architecture.md:1097-1101,1201-1205), while Feature 004 intentionally uses bounded direct event channels because Reliable Bus delivery can grow unbounded (docs/BACKLOG.md:256-276; gateway_backend.rs:292-324). This needs an explicit architecture amendment.

The PersistedPane architecture model omits shipped command history and EnvSnapshot (architecture.md:1611-1630 versus schemas/persist/session.vexil:21-108), and the schema still says snapshot persistence is unwired (session.vexil:93-98). ADR-0001/0002 retain superseded present-tense claims (docs/adr/ADR-0001-malt-stack-dependency-reversal.md:67-74; ADR-0002-gateway-canonical-mcp-adapter.md:15-39,80-98).

Feature 004 slow-consumer quickstart is Bash-only in a Windows-first repository (specs/004-command-lifecycle-events/quickstart.md:62-80). There is no root README defining supported commands, token handling, VNP trust boundary, isolation caveats, or source-of-truth order. The untracked vision note calls delivered history, lifecycle events, and responsive execution unimplemented; preserve it, but refresh it before treating it as design authority.

Correction: update governing architecture/schema sections and add ADR implementation-status addenda without rewriting rationale; provide PowerShell instructions or platform labels; add a concise root entry point.

## Recommended closure order

1. Close VNP authentication and client-scoped authority enforcement (A-01/A-08).
2. Make isolation fail closed, including restored Compat processes (A-02/A-06/A-11).
3. Repair CSPRNG/token lifecycle and real rate limiting (A-03 through A-05).
4. Implement process termination and true raw input; reject pane/elevation stubs (A-06/A-07/A-09/A-12).
5. Repair invariants, framing, and quality gates (A-13 through A-17).
6. Reconcile tracker, specs, architecture, ADRs, and quickstarts after semantics settle (A-18 through A-20).

## Evidence boundary

Passing workspace tests prove compilation and covered regressions only; they do not disprove the negative-path findings. No human acceptance, release proof, remote deployment, or external security assessment was performed.

