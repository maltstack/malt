# Data Model: CLI Isolation Flag

This feature adds no persisted entity or schema. It introduces command-side
values that map to the existing session-creation contract.

## CLI Isolation Selection

| Field | Type | Rules |
|---|---|---|
| `isolation` | optional isolation-tier argument | If present, it is exactly one of `bare`, `restricted`, `capped`, or `contained`. Missing or unsupported values are rejected before session creation. |

**State**:

1. Omitted: retain the existing session-creation request and expect Bare.
2. Selected: serialize the exact canonical lower-case tier and expect the
   matching reported tier.
3. Rejected: no request is made and the command exits unsuccessfully.

## Create-Session Request

| Field | Type | Rules |
|---|---|---|
| `name` | optional text | Existing behavior; may be combined with an isolation selection. |
| `isolation` | optional canonical lower-case tier | Included only when the operator selected `--isolation`; it is passed unchanged to the existing session authority. |

**Relationship**: One command invocation produces at most one
create-session request. The request is the only change-bearing interaction in
this feature.

## Created Session Result

| Field | Type | Rules |
|---|---|---|
| `id` | session identifier | Included in a successful user-visible creation result. |
| `name` | optional text | Included using the established fallback when absent. |
| `isolation` | reported tier | Must case-insensitively equal the selected tier, or Bare when no tier was selected, before the CLI reports success. |
| `state` and `pane_count` | existing summary fields | Deserialized unchanged; not used to choose the tier. |

**State transition**:

`requested` → `creation result received` → `verified success` when the tier
matches, otherwise `reported mismatch` and a non-successful command result.
The mismatch path does not issue a retry or deletion.
