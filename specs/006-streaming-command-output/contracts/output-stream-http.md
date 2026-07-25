# Contract: HTTP output stream

**Feature**: `specs/006-streaming-command-output/`

Deliberately shaped to match the existing lifecycle-event stream
(`GET /sessions/{id}/events`, feature 004) rather than inventing a second
streaming idiom on the same surface.

---

## `GET /sessions/{id}/output/stream`

**Scope**: `Read` (FR-017, R8). Observation, not interaction.

**Response**: `text/event-stream`, chunked, open until the client disconnects
or the session ends.

### Frame format

```
id: <sequence>
event: output
data: {"command_id":<u32>,"stream":"stdout"|"stderr","data":"<base64>","produced_at":<u64>}

```

- `id` is the chunk's session-wide `sequence`, which is what `Last-Event-ID`
  resumes from.
- `data.data` is **base64**. Output is arbitrary bytes and a multi-byte
  character may be split across chunks; lossy decoding at a boundary is not
  recoverable by the client (R6, FR-011).

### Gap frame

```
event: gap
data: {"reason":"retention_exceeded"|"subscriber_lagged","from":<u64>,"to":<u64>}

```

Emitted when a consumer resumes from a sequence no longer retained, or when a
subscriber falls behind. After a `subscriber_lagged` gap the connection is
closed — the subscriber is dropped, not accommodated (FR-009, FR-010).

**A gap is never optional.** Closing the stream without one, or omitting it
because the buffer is full, leaves the consumer believing it has a complete
picture. That is the failure this frame exists to prevent.

### Resumption

`Last-Event-ID: <sequence>` requests everything after that sequence.

- Retained → those chunks are replayed, then live delivery continues.
- Not retained → a `retention_exceeded` gap naming the lost range, then live
  delivery continues.
- Absent → delivery starts from the next chunk produced. It does **not**
  replay all retained output; a client wanting current state calls the
  existing output endpoint first.

### Status codes

| Code | When |
|---|---|
| 200 | Stream opened. |
| 401 / 403 | Missing or insufficient credential (`Read` required). |
| 404 | No such session. |

---

## Changed: `POST /sessions/{id}/exec`

Unchanged in shape. Two additions to the response body (R3):

| Field | Type | Meaning |
|---|---|---|
| `truncated` | `bool` | Output exceeded the reply cap. |
| `omitted_bytes` | `u64` | How much was left out. |

`output` remains the command's output up to the cap. **Truncation is stated,
never silent**; a caller that ignores the flag still sees output, and one that
checks it knows to use the stream instead.

Existing callers (`malt exec`, MCP `run_command`) keep working, since short
commands never reach the cap.

---

## CLI

```
malt watch <ID> --output [--resume-from <sequence>]
```

Extends the existing `malt watch` rather than adding a sibling command, so
there is one first-party consumer of session streams. It is also the thing
that will catch a defect the automated tests can miss — feature 004's parser
bug survived eight passing unit tests and was found by running `malt watch`.

---

## Compatibility

- Purely additive: one new route, two new optional response fields.
- No existing route changes shape, status, or scope.
- Clients that never call the stream behave exactly as before.
