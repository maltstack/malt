# Quickstart: Validating Authenticated Raw Input with Input Authority

**Feature**: 005-raw-input-authority

Runnable scenarios proving the feature works. Shapes are in
[contracts/input-and-authority.md](contracts/input-and-authority.md); field
semantics in [data-model.md](data-model.md).

## Prerequisites

- `cargo build --workspace` green, and all four gates passing (build, test, fmt, clippy).
- A daemon started fresh: `cargo run -p malt-bin -- daemon --port 7700`.
- The token from `~/.config/malt/api-token`. **It should no longer be printed to the console** — if you see it in the daemon's output, User Story 1 is not done.

## Scenario 1 — An unidentified client learns nothing (User Story 1)

Connect to the VNP port without a credential and confirm the connection is
refused *and* discloses nothing:

```bash
# VNP is HTTP port + 1
printf 'garbage\n' | timeout 5 nc 127.0.0.1 7701 | xxd | head
```

**Expected**: the connection closes without a session inventory in the
response bytes. Create a couple of named sessions first, then repeat — their
names must not appear anywhere in what comes back. This is the check that
matters: refusing *after* sending the inventory would pass a naive test.

Then confirm a legitimate client still works:

```bash
cargo run -p malt-bin -- new --name authtest
cargo run -p malt-bin -- attach 1     # should behave exactly as before
```

Verify the identification deadline, too: open a connection and send nothing.

```bash
timeout 60 nc 127.0.0.1 7701
```

**Expected**: the daemon closes it well before 60 seconds, and the daemon
keeps serving other clients normally throughout.

## Scenario 2 — An interactive command can be answered (User Story 2)

The core of the feature. In one terminal, run a command that blocks on input:

```bash
cargo run -p malt-bin -- exec 1 "read -p 'name: ' NAME; echo \"got=[\$NAME]\""
```

It should block. In another terminal, answer it:

```bash
cargo run -p malt-bin -- send 1 "world
"
```

**Expected**: the first command completes and reports `got=[world]`.

Then the three ways the old path corrupted input — each must survive:

```bash
# leading/trailing whitespace preserved
cargo run -p malt-bin -- exec 1 "read X; echo \"[\$X]\""
cargo run -p malt-bin -- send 1 "  padded  
"
# expect: [  padded  ]  -- not [padded]

# a bare newline is a real answer, not empty input to discard
cargo run -p malt-bin -- exec 1 "read -p 'continue? ' Y; echo \"answer=[\$Y]\""
cargo run -p malt-bin -- send 1 "
"
# expect: answer=[]  -- the command proceeds; it does not hang
```

**External processes** (the REPL/installer half of the story):

```bash
cargo run -p malt-bin -- exec 1 "cat"      # external, reads stdin
cargo run -p malt-bin -- send 1 "through-external
"
```

**Expected**: `cat` echoes the line. If it instead hangs or reads from the
daemon's own console, only the builtin half of User Story 2 is done.

**Confidentiality** — the requirement that exists because 003 and 004 shipped:

```bash
cargo run -p malt-bin -- exec 1 "read -s -p 'pw: ' PW; echo ok"
cargo run -p malt-bin -- send 1 "hunter2
"
cargo run -p malt-bin -- history 1 | grep -c hunter2   # expect 0
```

Also watch the event stream across the same exchange and confirm `hunter2`
never appears:

```bash
cargo run -p malt-bin -- watch 1
```

## Scenario 3 — One typist at a time (User Story 3)

Attach two clients to the same session, then have the non-holder try to send:

**Expected**: the holder's input is delivered; the non-holder is refused with
a reason naming the holder — not silently dropped, which would be
indistinguishable from a dead connection. Ask who holds authority and get a
definite answer.

To check for interleaving, have both send distinguishable payloads
concurrently to a waiting `read`. **Expected**: the command receives bytes
from exactly one of them, with none of the other's mixed in.

## Scenario 4 — Handover without stranding (User Story 4)

With client A holding authority and client B attached, have B claim it.

**Expected**: B's input is now accepted, A's is refused, and both are told.

Then the case that matters most — kill the authority holder abruptly (close
its terminal, or `kill -9`) while a command waits at a prompt:

**Expected**: another attached client can claim authority immediately and
answer the still-waiting prompt. No timeout, no restart, no stuck session.

## Automated verification

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Plus the conformance gate, because `mash` is modified by this feature:

```powershell
cargo build -p mash
$env:MASH = (Resolve-Path .\target\debug\mash.exe).Path
cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
# Expected: passed: 183, skipped unsupported: 3
```

A Smoosh regression is a blocker for this feature, not a note — `read` and
external-process stdin are POSIX surface.

Tests that must drive the real path, not a convenient shortcut:

- Handshake rejection through an actual socket connection, asserting the inventory is absent from the bytes sent — not by calling an auth function directly.
- Authority through the real VNP attach path. `AuthorityTracker` passes its own unit tests *today* while being unreachable from production; a test that calls it directly would prove nothing.
- Raw input through a genuinely blocked reader, asserting on what the command received.

## Known caveat

Identification uses the shared token, not peer credentials. That
authenticates "a process running as the user who started the daemon" — the
same boundary the Gateway already asserts, and a large improvement on no
check at all, but not per-process identity. `architecture.md` specifies peer
credentials, and `malt-platform` already models the transports that would
carry them; the migration is backlogged.
