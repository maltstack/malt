# Contract: Authenticated Connection, Raw Input, and Input Authority

**Feature**: 005-raw-input-authority | **Date**: 2026-07-25

Three surfaces change: the VNP handshake gains authentication, raw input gets
a path distinct from command submission, and authority becomes observable and
claimable.

## VNP connection

### Handshake

A client presents a credential as part of the opening exchange. The daemon
replies with the session inventory **only after** the credential validates.

| Condition | Behaviour |
|-----------|-----------|
| Valid credential | Handshake completes; inventory sent; a `client_id` is allocated and bound to the identity and its scope |
| Absent or invalid credential | Connection refused and closed. **No session inventory, no session count, nothing** — a refusal must not disclose whether any sessions exist |
| No credential within the identification deadline | Connection closed and its resources released |
| Too many unidentified connections in flight | New connections refused, so legitimate clients keep being served |

**Ordering requirement**: today the inventory is collected before the
handshake and sent inside it. That order inverts. This is the substance of
the contract, not an implementation note — a client must not be able to learn
what sessions exist by connecting.

### Attach

`AttachSession` carries a session identifier and a requested
`InputAuthority`. Both are now honoured rather than parsed and discarded:

| Condition | Behaviour |
|-----------|-----------|
| Identity may reach the session | Attach succeeds; the requested authority is applied |
| Identity may not reach the session | Refused. Naming a session id is not authorization |
| Session does not exist | Refused, distinguishably from "not permitted" |

## Raw input

### Sending input to a waiting command

Input is sent as **bytes**, over a path distinct from command submission. A
client states which it means; there is no heuristic.

| Outcome | Meaning |
|---------|---------|
| Accepted | Bytes written to the session's input channel, unmodified |
| Refused — not the authority holder | Another client holds input authority; includes who |
| Refused — input buffer full | The session is holding as much unread type-ahead as it will; retry after the command consumes some |
| Refused — session unreachable | Dormant or nonexistent, as with other session operations |

**Guarantees:**

- **Byte-for-byte.** No decoding, no trimming, no dropping of empty input. A bare newline is meaningful — it is the answer to a confirmation prompt. Bytes that are not valid text pass through unchanged. Leading and trailing whitespace is preserved; a password may contain it.
- **Not a command.** Input delivered this way never becomes an execution. It acquires no command id, produces no command-history entry, and emits no lifecycle events. Prompts routinely carry passwords, and both of those surfaces are readable at `Read` scope.
- **Type-ahead.** Input sent when nothing is reading is retained for the next read, up to a bound. Beyond the bound it is refused — never silently discarded, and never allowed to grow without limit.
- **No echo.** The daemon does not echo input. Whatever the command chooses to display appears through its normal output. A daemon that echoed would print passwords to every observer.

### `POST /sessions/{id}/send` (Gateway)

Existing route, changed meaning. It currently submits its payload as a *new
command execution* and waits up to 30 seconds for it to run. It becomes what
its name says: write these bytes to the session's input.

| Status | Condition |
|--------|-----------|
| `200` | Bytes accepted |
| `409` | Caller does not hold input authority (body names the holder), or the session is dormant |
| `429` | Input buffer full — retry |
| `404` | No such session |
| `401`/`403` | Missing credential, or scope below `Interact` |

**This is a behavioural change to an existing endpoint, not an addition.**
Any caller relying on `send` to run a command must use `exec`. Called out
here because it is the kind of change that silently breaks an integration.

## Input authority

### Observing

A client can ask who holds input authority for a session and get a definite
answer: a client identity, or nobody.

### Claiming

A client may claim authority. The claim succeeds immediately; the previous
holder is told it no longer holds. Consent is deliberately not required —
a holder that has stopped responding, or departed, would otherwise strand
the session (spec Assumptions, FR-018).

| Condition | Behaviour |
|-----------|-----------|
| Claimant is attached | Authority transfers; previous holder notified; all attached clients notified |
| Claimant already holds it | No-op. No notification storm to other clients |
| Claimant is not attached | Refused |

### Losing it

Authority is released when its holder detaches or disconnects, cleanly or
abruptly. No timeout, no grace period: the next attached client can claim
immediately. A session with no clients is claimable by whoever attaches next.

### Notification

Every attached client is told when authority changes hands, including the
gaining and losing parties. A client must never believe it can type when it
cannot — that produces input that vanishes, which is indistinguishable from
a broken connection.

## Not in this contract

- **Peer-credential identification.** `architecture.md` specifies identifying local connections by PID via peer credentials, and `malt-platform` already models `UnixSocket`/`NamedPipe` transports. VNP runs on loopback TCP, which has no peer credentials, so this feature authenticates with the existing token instead. The migration is backlogged; the seam here is transport-neutral so it is a swap rather than a redesign.
- **Terminal control.** Window resize signalling, job-control key handling, and raw versus cooked modes ride alongside input in a traditional terminal and are deliberately out of scope.
- **Per-client rate limiting of input.** The existing rate limiter is itself broken (audit A-05, a lifetime counter that never refills) and is backlogged separately.
