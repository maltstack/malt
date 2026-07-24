# Findings: Client/SDK Surface Audit for Agent-Facing Hardening

Date: 2026-07-24
Context: AGENTS.md's phased "Implementation Roadmap" is being retired in favor
of a correctness/hardening-first strategy. Explicitly prioritized: "one
excellent agent client or Gateway SDK." Explicitly paused: maltty polish,
plugin marketplace/lifecycle, remote deployments, exotic isolation tiers,
new FrameElement/UI variants, MCP-specific expansion, elaborate
observability, and maintaining multiple competing human clients. This audit
inventories the current client/SDK surface (`malt-gateway`, `malt-mcp`,
`malt-tui`, `maltty`, `malt-web`, `malt-bin`) against that goal, verifies
ADR-0002's "Gateway canonical, MCP adapter" claim against the actual code,
and ends with a recommendation and prioritized work list.

Method: read every route handler, the MCP server's `main.rs`, every client's
connection layer, and the CLI's command/client code directly — not just
AGENTS.md's descriptions of them. Confirmed vs. suspected is called out
explicitly throughout.

Builds on (does not re-derive) commits `750fd37..13d298c` and
`docs/adr/ADR-0002-gateway-canonical-mcp-adapter.md`: real exit codes now
flow through `exec`; `get_output` still returns `StyledGrid`; `command_id`
is still hardcoded to `0`; `send_input` still dispatches `RunCommand`
instead of writing to live stdin; `CommandBlock` is still unwired.

## 1. Current surface inventory

### `malt-gateway` — HTTP REST, axum 0.8

Routes (`crates/malt-gateway/src/server.rs:10-27`), all mounted with **no
auth or rate-limit middleware applied** (see §4):

| Route | Handler | Backend call |
|---|---|---|
| `GET /health` | `routes::health::health` | — (static `"ok"`) |
| `GET /sessions` | `routes::sessions::list` | `list_sessions` |
| `POST /sessions` | `routes::sessions::create` | `create_session` |
| `GET /sessions/{id}` | `routes::sessions::get` | `get_session` |
| `DELETE /sessions/{id}` | `routes::sessions::destroy` | `destroy_session` |
| `POST /sessions/{id}/exec` | `routes::sessions::exec` | `exec_command` |
| `POST /sessions/{id}/send` | `routes::sessions::send_input` | `send_input` |
| `GET /sessions/{id}/output` | `routes::sessions::output` | `get_output` |
| `GET /sessions/{id}/panes` | `routes::panes::list` | `list_panes` |
| `POST /sessions/{id}/panes/split` | `routes::panes::split` | `split_pane` |
| `DELETE /sessions/{id}/panes/{pane_id}` | `routes::panes::close` | `close_pane` |
| `POST /shutdown` | added ad hoc in `crates/malt-bin/src/daemon.rs:41-47`, not part of `build_router` | — |

That's 11 gateway routes + 1 CLI-added shutdown route — matches AGENTS.md's
"11 endpoints + /shutdown" count.

Concrete gaps found while reading `crates/malt-daemon/src/gateway_backend.rs`
(`DaemonBackend`, the only real `GatewayBackend` impl):

- `list_panes` (line 150-161) **ignores the session's actual panes** and
  always returns a single hardcoded `PaneResponse { id: 1, kind: "Shell",
  title: None, focused: true }` regardless of how many panes the session
  really has. Not previously flagged in BACKLOG.md.
- `split_pane` (line 163-176) is an explicit stub — comment says "Stub —
  split pane integration deferred," returns a fake `PaneResponse { id: 0,
  ... }` without touching the daemon.
- `close_pane` (line 178-180) is a no-op that always returns `Ok(())`.

So of 11 gateway routes, 3 pane-management ones (`list`/`split`/`close`) are
either wrong or stubbed. Session lifecycle (`list`/`create`/`get`/`destroy`)
and command execution (`exec`/`send`/`output`) are the only routes doing
real work today — which happens to be exactly the surface the demo scenario
needs, so this doesn't block it, but it does mean "11 endpoints" overstates
how much of the gateway is real.

Audience: `GatewayBackend` trait (`crates/malt-gateway/src/backend.rs`) is
generic — used today by exactly one real implementation (`DaemonBackend`)
and one test mock. Consumers today: `malt-bin` (CLI), `malt-mcp` (agents),
`malt-tui`/`maltty`/`malt-web` (`HttpConnection` polling mode only — see
§5). No consumer uses `list_panes`/`split_pane`/`close_pane` today; grepping
all clients turns up zero call sites for them outside gateway's own tests.

### `malt-mcp` — JSON-RPC 2.0 over stdio

`crates/malt-mcp/src/main.rs` is ~250 lines, single file, no daemon/VNP
dependency at all (`Cargo.toml`-level: only `serde_json` + `reqwest::blocking`
+ `anyhow`). Six tools (`tools_schema()`, lines 7-71), each a 1:1 HTTP proxy
via `dispatch_tool()` (lines 141-220):

| Tool | HTTP call |
|---|---|
| `list_sessions` | `GET {api_addr}/sessions` |
| `create_session` | `POST {api_addr}/sessions` |
| `run_command` | `POST {api_addr}/sessions/{id}/exec` |
| `get_output` | `GET {api_addr}/sessions/{id}/output` |
| `send_input` | `POST {api_addr}/sessions/{id}/send` |
| `destroy_session` | `DELETE {api_addr}/sessions/{id}` |

Every tool call returns `resp.text()` verbatim — i.e. the tool result an
agent sees is literally the Gateway's raw JSON body, unmodified. Audience:
AI agents that speak MCP over stdio, one HTTP round-trip per tool call, no
push/streaming, no session/connection state beyond the `MALT_API_ADDR`
env var.

### `malt-bin` — CLI, clap 4 + reqwest

Commands (`crates/malt-bin/src/cli.rs:14-45`): `daemon`, `start`, `stop`,
`status`, `list`, `new [--name] [--isolation]`, `attach [session_id]`,
`kill`, `exec`, `send`. `MaltClient` (`crates/malt-bin/src/client.rs`) wraps
`health`/`shutdown`/`list_sessions`/`create_session`/`destroy_session`/
`exec_command`/`send_input` — **notably, no `output`/`get_output` client
method exists at all.** `malt exec` prints `result.output` and exits with
`result.exit_code` (`main.rs:219-230`) — this is the one first-party
consumer that already gets the "right shape" (plain text + real exit code)
because it hits `/exec`, not `/output`. `malt attach` shells out to a
separate `malt-tui` binary (VNP mode) rather than consuming the Gateway
itself for rendering. Audience: human operator at a terminal, plus
scriptable automation via `exec`/`send`/`list`/`new`.

### `malt-tui` — ratatui 0.30 + crossterm 0.29

Three connection modes, all live in `crates/malt-tui/src/connection.rs`
behind one `DaemonConnection` trait:
- `MockConnection` — static 3-line demo, used when launched with no args.
- `HttpConnection` — polls `GET /sessions/{id}/output` every input-loop
  tick, parses the `StyledGrid` `rows` shape into `RenderCommand::DrawText`.
  Also has a `data.get("text")` plain-text fallback path (lines 225-250) —
  **this branch is dead code today**: `DaemonBackend::get_output` (§1 above)
  only ever emits `{"type": "StyledGrid", "rows": [...]}}`, never a `text`
  field, so this fallback has no live caller. Sends keys by converting
  `KeyEvent` back into raw terminal byte sequences and POSTing to `/send`
  (`key_event_to_text`, lines 85-143) — has no resize endpoint to call
  (`send_resize` is a no-op, line 269-271, comment: "HTTP API does not yet
  have a resize endpoint").
- `VnpConnection` — real bitpack Hello/HelloAck → AttachSession →
  InitialState → RenderBatch/FrameAck loop (lines 289-552). This is the
  mode `malt attach` actually launches (`--vnp` flag set in
  `malt-bin/src/main.rs:194-203`). Full typed protocol, no JSON.

12 tests, all in `tests/{style,render,input}.rs` — none exercise
`HttpConnection` or `VnpConnection` against a live or mock socket/HTTP
server; they test rendering/style/input-mapping logic in isolation.
Audience: humans (the VNP path is malt's real interactive terminal); the
HTTP polling path is effectively unused by any first-party flow today
(nothing launches `malt-tui` in HTTP mode) — it exists but is orphaned.

### `maltty` — wgpu 24 + winit 0.30 GPU client

`crates/maltty/src/renderer.rs::GpuRenderer::render()` (line 77-128) takes
`_lines: &[Vec<StyledSpan>]` — **the parameter is prefixed `_` and never
read.** The render pass clears the surface to a fixed dark color
(`r:0.05,g:0.05,b:0.07`) and presents — that's the entire frame. The
comment block at lines 118-124 spells out exactly what's missing: cosmic-text
font shaping, a glyph atlas texture, per-glyph quad instances, a
vertex/fragment shader. None of that exists. `App::render()`
(`crates/maltty/src/app.rs:44-46`) calls this renderer with `self.lines`,
which *is* populated correctly from `HttpConnection::poll_styled()` — the
data path works, only the draw call is unimplemented. Zero tests
(`crates/maltty/tests/` doesn't exist) — "build-only verification" per
AGENTS.md is accurate. **Concretely: maltty cannot display a single
character today.** It is not a partially-working terminal; it is a window
that stays black.

### `malt-web` — SvelteKit 2 + Svelte 5

`clients/malt-web/src/lib/api.ts` — plain `fetch()` wrappers for
`listSessions`/`createSession`/`getOutput`/`sendInput`/`execCommand`, no
typed error handling (a failed `execCommand` returns `''` via
`json.data?.output || ''`, silently swallowing errors — line 55). Single
page (`+page.svelte`) polls `/output` every 100ms, renders the `StyledGrid`
`rows` shape directly as styled `<span>`s, keeps a **client-side local-echo
input buffer** (`inputBuffer`, lines 43-66) that is never reconciled against
what the daemon actually echoes back — meaning if the daemon's own PTY/mash
echo behaves differently than the client's naive echo (it will, once real
programs with raw-mode input are involved), the visible text and the actual
shell state diverge. No resize handling at all — no viewport dimensions are
ever sent to the daemon. Zero test files (`**/*.test.*` glob: no matches).
Audience: casual human browser access — explicitly an "MVP" per AGENTS.md,
and the code confirms that framing (no tests, no error surfacing, no
reconciliation of client-echo vs. server state).

## 2. Is ADR-0002's "Gateway canonical, MCP adapter" true today?

**Yes — cleanly, with zero drift found.** `crates/malt-mcp/src/main.rs` has
no dependency on `malt-daemon`, `malt-protocol`, or any VNP/socket code —
its `Cargo.toml`-level deps are `serde_json`, `reqwest`, `anyhow` only (a
`resolve-library-id`-style check isn't needed here; the `use` statements at
the top of `main.rs` and the total absence of any `malt_*` crate import
confirm it directly). All six tools go through `reqwest::blocking::Client`
to `{api_addr}/...` — the exact same routes `malt-bin` and the HTTP clients
use. There is no bypass path anywhere in `malt-mcp` that talks to the
coordinator, the bus, or a raw socket.

Consequence of this cleanliness: **MCP inherits every Gateway defect
verbatim, with zero added value or added bugs.** `get_output`'s
`StyledGrid`-shape problem, `send_input`'s wrong-`RunCommand`-dispatch
problem, and `command_id: 0` all show up character-for-character in what an
MCP tool call returns, because MCP does no interpretation of the response at
all (`Ok(resp.text()?)`, verbatim in all six branches of `dispatch_tool`).
This is good news for the "fix it once, in the Gateway" strategy ADR-0002
proposes — there is no second copy of these bugs living in `malt-mcp` that
would need a separate fix.

**One divergence found, not previously documented:** `malt-mcp`'s
`create_session` tool (lines 153-162) hardcodes
`arguments.get("name")...unwrap_or("default")` — if an agent omits `name`,
MCP sends `{"name": "default"}` to the Gateway. The Gateway's
`CreateSessionRequest` (`crates/malt-gateway/src/types.rs:36-40`) treats
`name` as `Option<String>` and is perfectly happy with an *absent* field
(`malt-bin`'s own `CreateSessionRequest` in `client.rs` uses
`skip_serializing_if = "Option::is_none"` to omit it deliberately, see its
test `create_session_payload_preserves_legacy_and_selected_tier_shapes`).
So an MCP-created session with no name specified gets literally named
`"default"`, while a CLI- or curl-created session with no name gets `None`
(shown as `-` in `malt list`). Minor, but it's a real behavioral divergence
between two "same contract" consumers that ADR-0002's principle 4 ("First-
party clients... consume the same public contract") is meant to prevent —
worth a one-line fix (drop the `unwrap_or("default")`) whenever `malt-mcp`
is next touched, not urgent enough to break out as its own backlog item on
its own.

**No isolation-tier support in MCP's `create_session` tool at all** — its
JSON schema (lines 15-23) only declares a `name` property; there is no way
for an agent to request `Restricted`/`Capped`/`Contained` isolation through
MCP today, even though the Gateway route accepts it and `malt new
--isolation` exposes it on the CLI. Confirmed capability gap: MCP supports
a subset of what the Gateway/CLI support, not a superset or an equal set.

## 3. What "one excellent agent client or Gateway SDK" needs

Given the confirmed gaps this audit and the prior session both found —
`get_output`'s wrong shape, missing execution IDs, no lifecycle events, no
real stdin, unwired `CommandBlock` — an agent-ergonomic client needs, at
minimum:

1. **A plain/structured output read**, not `StyledGrid`, for programmatic
   consumption (BACKLOG.md P1 already scopes this: "add a plain-
   text/structured variant of `get_output` alongside the existing grid
   one").
2. **A real execution identity**: `command_id` that isn't `0`, returned
   from `/exec` and referenceable later — this is blocked on
   `CommandBlock` wiring, not a client-side concern.
3. **A lifecycle/event feed**, not just a blocking round-trip. This audit
   found something ADR-0002 didn't call out explicitly: **the three
   lifecycle message types the whole design leans on —
   `CommandStarted`/`CommandFinished`/`PromptReady`
   (`schemas/shell.vexil:9,18,27`) — are schema-defined and even have
   correct codec constants (`MSG_COMMAND_STARTED`=0x01,
   `MSG_COMMAND_FINISHED`=0x02, `MSG_PROMPT_READY`=0x03 in
   `crates/malt-protocol/src/codec.rs:27-29`, fixed in the 2026-07-24 codec
   audit), but grepping all of `malt-daemon/src` for where they're
   published finds nothing. Only `OutputChunk` (msg_type 4) is ever
   actually published on the bus** (`crates/malt-daemon/src/executor/
   session_thread.rs:314-320` and `:490-495`, both `domain: 1, msg_type: 4`
   — no other Shell-domain message is constructed or published anywhere in
   the crate). This means today, even internally, there is no
   started/finished/prompt-ready signal for anything to consume — an SDK
   built "on top of" these events would have nothing to subscribe to until
   this is wired, independent of any HTTP/MCP surface work. This is new,
   more specific evidence for the "genuinely new engineering" call already
   made in ADR-0002's implementation addendum, not a contradiction of it.
4. **A real stdin-write path**, distinct from `RunCommand` — confirmed
   still broken (`gateway_backend.rs:117-134`, `send_input` dispatches
   `SessionCommand::RunCommand` and discards the output).
5. **Resumption from a cursor/execution id after reconnect** — needs
   `CommandBlock`'s `VecDeque` (already non-destructively readable,
   `crates/malt-session/src/pane.rs:82-84`) actually populated, plus a
   Gateway route to read it by id/cursor. Nothing today reads
   `command_blocks()` outside its own tests (confirmed via the same grep
   ADR-0002 already ran — still zero hits).

**Recommendation: (a), a new thin `malt-gateway-sdk` Rust crate — not (b)
hardening `malt-mcp` into this role.** Reasoning:

- MCP's actor model is wrong for this even once the Gateway is fixed. This
  was already the ADR-0002 finding for the `get_output` shape problem
  specifically ("external agent remotely driving a persistent session,"
  not "editor hosting an embedded agent's own UI") — but it generalizes:
  MCP's tool-calling round-trip has no first-class notion of a long-lived
  subscription to a lifecycle-event stream, which is exactly what step 3
  above needs. Bending MCP to carry that (SSE-over-MCP, polling
  `get_output` in a loop) is exactly the "10-32x more expensive in tokens,
  ~28% failure rate" pattern ADR-0002 already cites as the empirical
  argument against MCP-first design.
- `malt-mcp` is 250 lines with zero internal-crate dependencies by design
  (confirmed in §2) — it is *already* the disposable adapter ADR-0002
  phase 7 wants it to be. Investing in it now, before the Gateway contract
  is stable, means re-doing that investment when MCP's own spec moves
  again (already flagged as in-progress: "2026-07-28 spec revision").
  Nothing about today's `malt-mcp` code resists being thrown away later —
  that's a feature of the current design, not a gap to close.
- A native Rust SDK crate can wrap the *fixed* Gateway HTTP contract with
  the right types once items 1-5 above land — typed `ExecResult` with a
  real `command_id`, a typed event-subscription API (long-poll or SSE) over
  the lifecycle feed once it's actually published, a typed stdin-write
  call, and a resume-from-cursor call. It becomes the thing `malt-bin`
  itself should also depend on (ADR-0002 principle 4 — "first-party
  clients consume the same public contract"), closing the gap found in
  §1 where `malt-bin` doesn't even have an `output` client method today and
  in §2 where MCP's session-naming default silently diverges from the
  CLI's. One typed crate used by `malt-bin`, `malt-tui`'s HTTP mode, and
  (thin) `malt-mcp` alike removes the class of bug found in §2 by
  construction, rather than requiring every consumer to independently get
  the JSON shape right.
- This is sequenced *after*, not instead of, Gateway Phase 3/4 fixes (real
  `command_id`, `CommandBlock` wiring, plain-output variant, stdin path,
  lifecycle events actually published). Building the SDK before those land
  would just typed-wrap today's known-wrong shapes.

## 4. Auth/scope fit for agent use

**The scope model cannot be evaluated for "is Interact sufficient" because
no scope is enforced on any route at all today — this is more fundamental
than a scope-design question.**

Evidence: `AuthContext`, `AuthScope` (Monitor < Read < Interact < Admin),
and `TokenStore` all exist and are well-tested in isolation
(`crates/malt-gateway/src/auth.rs`, `crates/malt-gateway/tests/auth.rs` — 8
tests, all exercising the struct directly). But:

- `build_router()` (`crates/malt-gateway/src/server.rs:10-27`) never
  extracts an `Authorization` header, never constructs an `AuthContext`,
  and applies no `axum::middleware` / `tower::Layer` of any kind. Every
  route handler's signature is `State(backend)` plus path/body extractors
  only — there is no auth extractor anywhere in
  `crates/malt-gateway/src/routes/*.rs`.
- `crates/malt-bin/src/daemon.rs:29-31` calls
  `token_store.load_or_generate_default()` and **prints the token to
  stdout** ("API token: {default_token}") — implying to an operator that
  the token means something — but the `token_store` variable is never
  passed into `build_router` or attached to the router in any way
  (confirmed: `build_router(backend)` at line 41 takes only `backend`).
  The token is generated, persisted to `~/.config/malt/api-token`, and
  printed, and then completely unused.
- `RateLimiter` (`crates/malt-gateway/src/rate_limit.rs`) has the same
  story: real token-bucket logic, 4 tests exercising it directly
  (`crates/malt-gateway/tests/rate_limit.rs`), and zero call sites outside
  its own test file anywhere in the workspace — not constructed in
  `daemon.rs`, not referenced in `server.rs`.
- Every `GatewayError` variant that would represent an auth/rate failure
  (`Unauthorized`, `Forbidden { required }`, `RateLimited`) has a correct
  `IntoResponse` mapping (401/403/429,
  `crates/malt-gateway/src/error.rs:43-64`) but is **never constructed
  outside tests** — grepping the whole gateway crate for these variants
  outside `error.rs` and its own tests finds no call site that returns
  them.

**Net effect: any process that can reach the daemon's HTTP port has full
`Admin`-equivalent access to every route, with or without a token, local or
remote.** This is a materially bigger issue for agent use than "is Interact
enough" — an agent (or anything else on the network path) gets destroy/kill
capability it never asked for and the operator has no way to restrict it,
because the restriction mechanism, while fully built and tested in
isolation, was never wired into the one router that matters. This matches
and sharpens ADR-0002 phase 5 ("auth scopes actually enforced on routes —
they exist but aren't consistently wired in") and Phase D/I of the old
roadmap — the correct framing is "0% wired," not "inconsistently wired."

Once wiring happens, `Interact` (per architecture.md's table: `read` +
`POST /send`, `POST /exec`, `POST /session/create`, session attach) does
look sufficient for the demo-lens scenario's agent-side steps (create
session, exec, poll/read output, send input) — no red flag was found
requiring `Admin` for anything in the 9-step scenario *except* one
structural gap: the scope table (architecture.md line 1184-1189) has no
explicit line for "destroy this session I created" — `admin` is listed as
"interact + session destroy." If an agent is expected to clean up its own
session on disconnect (demo step "session survives client disconnection" —
implying deliberate detach is a distinct, non-destructive operation), that
part is fine at `Interact`; but if a still-unbuilt "agent tears down its
own session when truly done" flow is ever wanted, that currently requires
`Admin` per the design table, which would be a real design smell (agents
needing elevated scope for routine self-cleanup) worth deciding
deliberately rather than defaulting into.

## 5. Redundant client surfaces — which one should be canonical

**`malt-tui` (VNP mode specifically) should be the single reference human
client. Neither `maltty` nor `malt-web` should receive further investment
beyond what already exists.**

Evidence ranking, most to least complete:

1. **`malt-tui`, VNP mode**: the only client anywhere in the repo using the
   real, typed, bitpack VNP protocol end-to-end (full
   Hello/HelloAck/AttachSession/InitialState/RenderBatch/FrameAck cycle,
   `crates/malt-tui/src/connection.rs:289-552`). It is what `malt attach`
   — malt's own primary entry point — actually launches
   (`crates/malt-bin/src/main.rs:193-203`). 12 tests. This is not close:
   it's the only client that isn't reading JSON off an HTTP polling loop.
2. **`malt-tui`, HTTP mode**: real code, but orphaned — nothing in the
   repo launches `malt-tui` in `--api-addr`-without-`--vnp` mode today
   (`main.rs:44-57`'s branching always prefers `--vnp` when both are
   theoretically available, and `malt-bin` never omits `--vnp`). Contains
   a dead fallback branch (the `text` field parse, §1). Candidate for
   deletion or explicit "kept for manual testing" documentation, not
   further investment.
3. **`malt-web`**: honestly labeled "MVP" and the code backs that up — no
   tests, silent error-swallowing (`|| ''`), an un-reconciled client-side
   echo buffer that will visibly diverge from real shell state under any
   raw-mode program, no resize support at all. Fine as a "look, it works in
   a browser" demo; not a foundation to build on without first fixing the
   echo-reconciliation problem, which is a real design task (it needs to
   either stop local-echoing and trust the server round-trip, matching
   the VNP model's architecture, or properly diff against server state).
4. **`maltty`**: cannot render a single character today (§1). Every other
   dimension (window creation, resize, input mapping, HTTP polling data
   path) works — only the one thing a terminal client exists to do
   (display text) is unimplemented. This is exactly the kind of "GPU
   client polish" work the new strategy says to pause, and the evidence
   supports pausing it hard: there's no partial credit to protect, and no
   quick win available (text rendering needs the full cosmic-text
   atlas/shader pipeline described in its own TODO comment, not a small
   patch).

Recommendation: freeze `maltty` and `malt-web` as-is (no bug fixes, no
feature work) until/unless the hardening push is complete and a genuine
need for a second human surface reappears. Point any human-facing effort
at `malt-tui`'s VNP mode, and consider deleting `malt-tui`'s HTTP mode and
`HttpConnection` struct entirely once the plain/structured `get_output`
work lands anyway (it would need rewriting regardless, since it currently
depends on parsing `StyledGrid` `rows`).

## Prioritized work items

Severity: **P0** blocks the demo-lens scenario outright, **P1** real gap
worth closing soon, **P2** smaller/cleanup. Effort is rough (S = hours,
M = a day or two, L = multi-day design + implementation).

1. **[P0, L]** Wire `TokenStore`/`AuthContext`/`RateLimiter` into
   `build_router` as real axum middleware — today the daemon prints a
   token that does nothing (§4). Unblocks: demo step "temporary input
   authority handoff" implicitly assumes *some* notion of who's allowed to
   claim authority — that has no enforcement layer to build on until this
   exists. Also unblocks safely exposing the Gateway beyond localhost at
   all, which every later SDK/agent-facing work implicitly assumes is
   eventually wanted.
2. **[P0, L]** Publish `CommandStarted`/`CommandFinished`/`PromptReady` on
   the bus from the daemon's command-execution path (they're schema- and
   codec-ready, just never constructed — §3). Unblocks: demo steps
   "structured progress," "command requests input or fails," and "agent
   resumes from the last execution event" all need *some* lifecycle event
   to exist before an SDK can expose it.
3. **[P1, M]** Fix `send_input` to write to a live process's real stdin
   instead of dispatching another `RunCommand` (confirmed wrong,
   `gateway_backend.rs:117-134`; carried over from before this audit, still
   unfixed). Unblocks: "command requests input" and any REPL/interactive
   demo step.
4. **[P1, M]** Wire `CommandBlock` construction + a real `command_id`
   into the `RunCommand`/`exec` path (carried over, still unfixed).
   Unblocks: "agent resumes from the last execution event," "daemon
   restarts and restores session history."
5. **[P1, S]** Add a plain/structured `get_output` variant alongside the
   grid one (carried over, still unfixed). Unblocks: "human attaches, both
   see the same authoritative state" only works well for the *agent* side
   if it isn't parsing `StyledGrid` by hand.
6. **[P1, M]** Fix `list_panes`/`split_pane`/`close_pane` in
   `DaemonBackend` — currently a fake single-pane response, a stub, and a
   no-op respectively (§1, newly found). Doesn't block the 9-step demo
   directly (single-pane scenario) but is a real correctness gap in a
   route surface an SDK would otherwise expose as if it worked.
7. **[P2, M]** Build `malt-gateway-sdk` (Rust crate) once items 2-5 land —
   typed wrapper for session CRUD, exec, plain-output read, lifecycle-event
   subscription, stdin write, and cursor-based history resume. Point
   `malt-bin` and a rebuilt thin `malt-mcp` at it (§3 recommendation).
8. **[P2, S]** Fix `malt-mcp`'s `create_session` name-defaulting divergence
   from the CLI's (`unwrap_or("default")` vs. omitting the field, §2) —
   trivial once someone's back in that file for the SDK work in item 7;
   not worth a standalone session.
9. **[P2, S]** Delete or explicitly document-as-manual-testing-only
   `malt-tui`'s HTTP connection mode and its dead `text`-field fallback
   branch (§5) — it's unused by any real launch path today and will need
   rewriting once item 5 changes `get_output`'s shape anyway.
10. **[P2, S]** Freeze `maltty` (§5) — no further work planned; this is a
    documentation/decision item (record the freeze somewhere durable, e.g.
    AGENTS.md's client section) rather than a code change.
