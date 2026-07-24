# Quickstart: Verify Responsive Session Control During Execution

Run these checks after implementing the plan. The commands assume PowerShell
from the repository root.

Use the [data model](./data-model.md) for state and ownership invariants and the
[session-control contract](./contracts/session-control.md) for exact observable
outcomes.

## 1. Re-establish the workspace baseline

The project session ritual requires a fresh build and test pass before resumed
work:

```powershell
cargo build --workspace
cargo test --workspace
```

Record any pre-existing failure before attributing it to this feature.

## 2. Run focused daemon tests

```powershell
cargo test -p malt-daemon --test session_thread
cargo test -p malt-daemon --test coordinator
cargo test -p malt-daemon --test gateway_backend
cargo test -p malt-daemon --test vnp_listener
```

The focused suite must prove:

- a command lasting longer than current attach/output waits does not block
  attach, styled/plain observation, resize, ack, key evaluation, snapshot, or
  detach;
- every specified control completes within one second in 100 consecutive
  trials under normal local load;
- capacity 1 accepts one active plus one pending request and explicitly rejects
  the next request;
- concurrent Gateway and editor submissions share one acceptance order;
- 1,000 multi-command trials are exactly once, FIFO, and non-overlapping;
- state-changing commands hand the completed state to their successor;
- successful, failing, stderr-producing, and high-output results match idle
  execution;
- high-output finalization services controls between bounded slices without
  exposing a partial view;
- worker failure leaves control responsive and gives active/pending work one
  explicit terminal outcome;
- busy last-detach preserves work and stays Active, while idle last-detach
  persists and becomes Dormant;
- shutdown intake closes promptly and reaping does not hold the global
  coordinator mutex;
- an indefinitely busy session does not delay another session.

## 3. Run Gateway contract tests

```powershell
cargo test -p malt-gateway
cargo test -p malt-daemon gateway
```

Verify the existing success envelopes are unchanged and the new failures map
exactly as follows:

| Condition | Status | Code |
|---|---:|---|
| Pending execution capacity exhausted | 503 | `execution_queue_full` |
| Execution worker unavailable | 503 | `execution_unavailable` |
| Session execution intake closing | 409 | `session_shutting_down` |

Also verify that HTTP 429 remains reserved for `rate_limited`.

## 4. Verify protocol and persistence compatibility

```powershell
cargo test -p malt-protocol
cargo test -p malt-daemon vnp
cargo test -p malt-daemon store
```

Confirm:

- no VNP golden encoding changes;
- attach still receives the existing consistent `InitialState`;
- no new execution or queue message exists;
- snapshots during execution return the last finalized `PWD`, `SHELL`, layout,
  and presentation state promptly;
- no active/pending request, command counter, result, or full `EnvSnapshot` is
  persisted;
- `schemas/persist/session.vexil` is unchanged by this feature.

## 5. Re-run shell conformance on native Windows

```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
```

Expected native-Windows result: 183 passed and 3 unsupported tests skipped.
This confirms that moving the original `Env` to one worker did not change MASH
semantics.

## 6. Run the final workspace gate

```powershell
cargo build --workspace
cargo test --workspace
```

Completion requires every test to pass. Local fixtures prove repository
behavior only; they do not replace live acceptance or Gateway-auth prerequisite
evidence.
