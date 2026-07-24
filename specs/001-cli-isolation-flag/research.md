# Phase 0 Research: CLI Isolation Flag

## Decision: Use a typed CLI isolation value with four canonical spellings

The `malt new` command will accept only `bare`, `restricted`, `capped`, and
`contained`. A typed command argument performs validation before the client can
send a session-creation request.

**Rationale**: The protocol schema fixes the same four tiers, while the CLI
needs a stable, shell-friendly lower-case contract. Early validation meets the
invalid and missing-value requirements without relying on a permissive remote
parser.

**Alternatives considered**:

- Pass arbitrary strings to the gateway: rejected because unknown values can
  currently be treated as Bare by the daemon gateway.
- Accept title-case and mixed-case aliases: rejected because they expand the
  public contract without user value and make command documentation ambiguous.

## Decision: Preserve omission rather than explicitly sending `bare`

When `--isolation` is absent, the CLI will retain its existing session-creation
payload behavior. When present, it will add the canonical lower-case
`isolation` value alongside the optional name.

**Rationale**: Omission already produces the established Bare default. Keeping
that request shape avoids an unnecessary behavior change for existing scripts
and lets the option be an additive capability.

**Alternatives considered**:

- Always send `"bare"`: rejected because it changes the established default
  request despite no user selection and has no functional benefit.

## Decision: Validate the reported session tier before printing success

The new-command handler will derive the expected tier (the supplied tier, or
Bare when omitted), compare it case-insensitively with the session result, and
only then print the creation result. Success output will include the reported
tier.

**Rationale**: Existing session responses use title-case debug-style tier
names, whereas the command input is lower-case. Case-insensitive comparison
preserves a precise four-tier match without coupling the CLI to presentation
casing. A mismatch is an error state, not a success state.

**Alternatives considered**:

- Trust the response without comparison: rejected because the command could
  represent an unintended execution boundary as a successful request.
- Retry at Bare after a mismatch or error: rejected because it would silently
  weaken the operator's request.
- Delete a possibly created session after a mismatch: rejected because the
  remote authority may already have created a session and automatic deletion
  could destroy work that requires operator inspection.

## Decision: Keep gateway and platform-enforcement remediation out of this feature

The feature will consume the existing `POST /sessions` integration and report
its result. It will not modify direct gateway handling of arbitrary callers,
the daemon's platform-capability decision, or process-containment behavior.

**Rationale**: The backlog defines this as exposing an already wired
gateway/daemon capability. Changing authority-side validation or platform
enforcement is a separate security/architecture change, with effects beyond
the primary CLI workflow. The CLI remains fail-closed in its own success
reporting and preserves any returned creation failure.

**Alternatives considered**:

- Repair all gateway parsing and capability checks in this work: rejected as a
  scope jump beyond exposing the CLI flag; it needs its own specification and
  platform-level proof.

## Existing Integration Evidence

- `crates/malt-bin/src/cli.rs` currently defines `malt new --name`; its parser
  tests provide the closest unit-test home for the typed option.
- `crates/malt-bin/src/client.rs` already creates sessions through the gateway
  and deserializes an `isolation` field in `SessionData`.
- The gateway request already has optional `name` and `isolation` fields, and
  the daemon coordinator persists the selected protocol isolation tier.
- The no-subcommand workflow has a separate creation call and is explicitly
  outside the planned edits.
