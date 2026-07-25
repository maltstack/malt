# Spec 005 quickstart verification — raw input and input authority

**Date:** 2026-07-25
**Feature:** `specs/005-raw-input-authority/`
**Daemon:** built from this worktree, HTTP on 7960, VNP on 7961

This records what was actually observed running the product, not what the
code appears to do. It exists because 003 and 004 both had defects that only
a live run exposed, and because this feature's own history includes three
rounds of confidently wrong conclusions drawn from reading code and from
tests whose controls did not exercise the path being claimed.

## What was verified, and how

### Scenario 1 — the interactive transport refuses the unidentified

A raw TCP socket connected to the VNP port and sent nothing.

```
connected, sent nothing -> 0 bytes disclosed before close
```

**Zero bytes**, then the server closed the connection. Before this feature the
same connection received the session inventory. The assertion is byte-level on
purpose: "the handshake failed" is compatible with having disclosed the
inventory first, so counting bytes is the only claim worth making.

### Scenario 2 — a client answers a prompt

```
prompt answered -> 'answered=[s3cret with spaces]'
secret in history: False
```

The bytes reached a command blocked in `read`, and the answer does not appear
in command history. That second check matters because features 003 and 004
both shipped surfaces that record command text at Read scope; routing a prompt
answer through command submission would have published a password into two
durable, readable places.

### Scenario 2b — end-of-input terminates a consuming command

```
wc -l -> '3'
```

`wc -l` consumes to the end of input, so it can only finish if end-of-input is
deliverable. This is the check that streaming tool input did not reintroduce
the hang it was meant to remove.

### Scenarios 3 and 4 — authority

Verified over **real TCP against the real listener**, in
`crates/malt-daemon/tests/vnp_listener.rs`, rather than through HTTP:

- `attaching_over_the_wire_applies_the_requested_authority`
- `attaching_as_an_observer_over_the_wire_does_not_take_authority`
- `dropping_the_connection_releases_authority` — the socket is dropped with no
  `DetachSession`, which is what an abrupt client death looks like to the
  daemon; the test then polls until authority is released, asserting it
  happens without a timeout or grace period.

Plus, over HTTP against the live daemon:

```
holder with nobody attached: {'holder': None}
input accepted when unheld : True
```

which confirms an unattached session stays answerable (FR-018).

## Honest limits of this verification

- **Scenarios 3 and 4 were not driven by two interactive TUIs.** They were
  driven by two real VNP clients over real TCP (integration tests) and by the
  HTTP surface. Speaking bitpack VNP from a throwaway script is not practical,
  so the wire-level tests are the strongest evidence available here. What has
  *not* been exercised is a human watching two terminals see each other's
  authority notices.
- **The TUI notice is untested end-to-end.** `show_notice` is drawn from the
  main loop on `take_authority_change`, and the decode path has no automated
  coverage of the rendered result.

## Note on a passing observation

`read -r PW` trimmed the surrounding spaces from `"  s3cret with spaces  "`,
which is correct POSIX for the default `IFS`. It is *not* the separate,
pre-existing `IFS=` bug recorded in `docs/BACKLOG.md`, which is about
`IFS=` being treated as unset. Delivery byte-fidelity is proven independently
by the `SessionInputChannel` unit tests.
