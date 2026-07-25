# Vision: Durable Links to Sessions and Executions

> **Status:** Vision note, not a spec. Not scheduled, not in `docs/BACKLOG.md`.
> Captures a design direction worth protecting while backlog items 2–9 are
> built, not a proposal to build now. Revisit once execution IDs and
> persistent execution history exist (see "Dependencies" below).

## The idea

An agent that reports on background work today can only describe it in
prose ("I started the build, it's still running"). There's no way to hand
the user something that opens directly onto the live state of that work.

If MALT sessions, panes, and executions were addressable resources with
stable identifiers, an agent could instead emit a link that resolves to the
actual object:

```
malt://execution/exe_01JQ8K...
```

A client resolving that link would attach as an observer, focus the
relevant pane, scroll to the execution's command block, and continue
streaming its output — without stealing input authority. A completed
execution resolves the same way, as a permalink to the recorded run rather
than a dead handle to an exited process.

This turns terminal state into something that can be pointed at and shared,
the way a URL points at a document. It's a natural extension of two things
MALT already asserts about itself: that the daemon, not the client, is the
authority on session state, and that the Gateway should expose resources,
not just RPC actions (`docs/design/architecture.md`, §1, Core Principle 4).

## Why this depends on owning the stack

A terminal client built on top of someone else's shell and emulator has no
durable object to point at — only a PID and a byte stream, both ephemeral
and process-local. MALT can do this because the daemon already outlives
the render surface and already assigns stable identity to session and pane
(`SessionId`, `PaneId` in `malt-protocol`). An execution link is the same
pattern extended one level deeper: identity for a unit of work, not just
for the container it ran in.

## Resource model

| Resource    | Identity today                              | Durable across daemon restart? |
|-------------|----------------------------------------------|----------------------------------|
| Session     | `SessionId(u32)`, exists now                  | Yes — session store persists it |
| Pane        | `PaneId(u32)`, exists now                     | Yes, as part of session state |
| Execution   | Does not exist as a first-class resource yet | No — not yet persisted |

The command block ring buffer in `malt-session` is the closest thing to an
execution record today, but it's in-memory and scoped to a live session,
not a durable, globally addressable object with its own ID, exit code, and
event history. Making executions addressable means promoting that concept
to a real resource — which is already implied by two items already on the
priority list, not new scope:

- **Priority 2** (real exit codes and execution IDs) — this is the
  identifier a link would target.
- **Priority 4** (persistent execution history) — this is what makes the
  link still resolve after the process, or the daemon, has restarted.

Priority 3 (command lifecycle events) and priority 9 (session restoration)
are what let a client answer "is this still running, and where did it go"
when a link is opened later. None of this needs new architecture; it needs
those items to be built with addressability as an explicit acceptance
criterion, not as a side effect.

## Illustrative URI shape

Not finalized — the exact scheme is a detail to settle at spec time, not
now:

```
malt://execution/exe_01JQ8K...          # local resolution via local daemon
https://<gateway-host>/e/exe_01JQ8K...  # remote resolution via API Gateway
```

Fragment addressing for finer-grained navigation (`#live`, `#stderr`,
`#event-284`) is a plausible extension once execution event history is a
real, ordered, durable stream — it falls out of priorities 3 and 4 rather
than requiring separate design.

## Permission semantics: reuse what exists, don't invent

A shared link must never silently grant more than observation. MALT
doesn't need a new permission model for this — the Gateway already has an
ordered scope hierarchy (Monitor < Read < Interact < Admin, in
`malt-gateway`'s auth module). A link's default capability should be
Monitor: attach and stream, no input authority. Claiming control, sending
input, or cancelling should require the same Interact/Admin scopes the
Gateway already enforces, surfaced as an explicit action in the client
rather than implied by opening the link.

## Addendum: the agent-facing execution contract

Addressability (above) is about pointing at an execution from the outside.
There's a matching question about how an agent *consumes* one from the
inside, and it's the same resource model viewed from the other side, not a
separate idea.

Today an agent's only interface to a command is a blocking call: submit,
block, get text back. That's a tool-contract property, not a law of nature
— once an execution is a durable resource with its own ID, the natural
shape is non-blocking:

```
start execution → get execution_id immediately → do other work →
observe status/events when useful → block only when the result is
actually needed (and only until then)
```

This isn't new scope either. It's the external shape of **priority 0b**
(decoupling command execution from the session's single blocking dispatch
thread) once that's done — 0b is what makes non-blocking execution possible
internally; this is what it should look like at the boundary an agent
actually calls. Concretely, a `wait` operation that blocks on the *caller's*
terms —

```
GET /v1/executions/{id}/events?after=284&wait_ms=10000
```

— returning when a new event arrives, the execution finishes, input is
required, or the timeout elapses (a timeout is not a failure; it returns
the current state and cursor) is a design constraint on **priorities 3**
(lifecycle events) and **4** (persistent history), not an extra feature:
build that event log with sequence numbers and a blocking-read-with-cursor
from the start, because retrofitting a cursor onto an event stream that
wasn't built with one is the kind of thing that's painful later and cheap
now.

Worth flagging as a real gap, not just a future nicety: even once the
internals are decoupled, MALT's current agent-facing surface (the MCP
tools `run_command`, `get_output`, `send_input`) is still shaped like a
blocking call. Non-blocking isn't purely an internal fix to priority 0b —
it's also a tool-contract change that hasn't been scoped anywhere yet.

## Deferred, not excluded

Two things came up alongside this that are genuinely part of what MALT's
resource model eventually supports — they're left out of this note not
because they're wrong, but because pulling them in now would blur one
buildable idea into a survey of everything the model could someday do:

- **Semantic/materialized views** (`GET /executions/{id}/view?select=status`
  returning something like `{"phase": "compiling", "completed_units": 41,
  "total_units": 63}` instead of raw output) — a real idea, but it means
  MALT understanding the specific output semantics of arbitrary tools
  (cargo's progress format, a given test runner's, etc.), which is
  effectively a parser-per-tool commitment. That's a materially bigger
  surface than the event log itself, and it's the kind of elaborate,
  open-ended observability work ADR-0003's correctness-first pivot
  deliberately paused — not because it's a bad idea, but because it isn't
  the current priority. Worth revisiting once the raw event log is durable
  and sequenced, since at that point a materialized view is "just" a
  projection over it.
- **Hosted relay / temporary share links** (`relay.malt.dev/share/...`) —
  implies a multi-tenant, network-exposed relay service, materially larger
  than local or gateway-mediated resolution. Not a fit for a first version.
- **Rich unfurl cards in chat/IDE surfaces** — a client-side presentation
  concern layered on top of a resolvable URI; doesn't affect whether the
  resource model or scheme itself is sound.

## When to revisit

Once priorities 2 (execution IDs), 3 (lifecycle events), and 4 (persistent
history) are implemented, this becomes specable as a Gateway resource
addition plus a client-side URI handler. At that point it's a normal
Spec Kit feature (`/speckit-specify`), not a vision note.
