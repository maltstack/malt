# Quickstart: Streaming Command Output

**Feature**: `specs/006-streaming-command-output/`

Runnable validation for each user story. **Every scenario starts from a real
command producing real output** — that is deliberate. The recurring defect in
this feature family has been a delivery path verified by injecting values
into it directly while nothing real ever reached it (Gateway auth,
`AuthorityTracker`, the tool stdin slurp). A scenario here that does not start
a command proves nothing.

## Prerequisites

```bash
cargo build --workspace
```

Start a daemon on a port that is not in use:

```bash
./target/debug/malt daemon --port 7970
```

The API token is read from `~/.config/malt/api-token`; `malt` subcommands pick
it up automatically. `curl` examples need it explicitly.

---

## Scenario 1 (US1) — output arrives before the command ends

Run a command that prints, waits, then prints again:

```bash
malt exec <ID> 'echo first; sleep 5; echo second'
```

While it is still running, from another shell:

```bash
malt output <ID>
```

**Expected**: `first` is visible **before** the command finishes, and the call
returns immediately rather than blocking for the remaining seconds.

**Before this feature** both would fail: the second call returns nothing until
the first completes.

---

## Scenario 2 (US1) — output survives a failing command

```bash
malt exec <ID> 'echo produced-before-failure; sleep 2; false'
```

**Expected**: exit code 1, and `produced-before-failure` was already delivered
to a watcher before the failure — not lost, and not replaced by the failure
report.

---

## Scenario 3 (US3) — an agent consumes and resumes

```bash
malt watch <ID> --output
```

with a command running in another shell. Interrupt the watcher mid-command,
note the last sequence it printed, then:

```bash
malt watch <ID> --output --resume-from <sequence>
```

**Expected**: every chunk between the two runs is received exactly once.
Verify **by content, not by count** (SC-003): concatenate what both runs
received and compare against the command's full output byte-for-byte.

Also check the raw stream directly, since a CLI bug can mask a protocol bug —
feature 004's parser dropped every frame while eight unit tests passed:

```bash
curl -N -H "Authorization: Bearer $(cat ~/.config/malt/api-token)" \
  http://127.0.0.1:7970/sessions/<ID>/output/stream
```

---

## Scenario 4 (US3) — a stalled subscriber is told it lagged

Open the stream, stop reading from it (suspend the reader; do not close the
socket), and run a command producing far more output than the subscriber
buffer holds.

**Expected**: the stalled subscriber receives a `gap` frame with
`subscriber_lagged` and is then disconnected. The command completes at normal
speed, and any other subscriber receives everything.

**Expected NOT**: the command slowing down, the session becoming
unresponsive, daemon memory growing without limit, or — worst — the
subscriber being dropped with no gap frame, which leaves it believing it saw
everything.

---

## Scenario 5 (US2) — an attached human sees it live

Attach a TUI:

```bash
malt attach <ID>
```

From another shell, run a command producing output over several seconds:

```bash
malt exec <ID> 'for i in 1 2 3 4 5; do echo line-$i; sleep 1; done'
```

**Expected**: the attached view updates several times during the command, not
once at the end.

**Note**: the known terminal-grid "staircase" defect (`docs/BACKLOG.md` P0)
lives in this path and will be *more* visible now. It is out of scope here —
do not mistake it for a regression introduced by this feature, and do not fix
it inside this feature (Principle IX).

---

## Scenario 6 (US2) — two clients see the same thing

Attach two clients, run one output-producing command, and compare what each
displays.

**Expected**: identical content; neither sees output the other does not.

---

## Scenario 7 (US4) — a built-in utility streams

```bash
malt exec <ID> 'cat' &
malt send <ID> 'first line
'
malt output <ID>          # expect: first line, before any further input
malt send <ID> 'second line
'
malt eof <ID>
```

**Expected**: `first line` is observable before `second line` is sent.

---

## Scenario 8 — byte fidelity and bounded memory

Non-text and split multi-byte output:

```bash
malt exec <ID> 'printf "caf\303\251\n"; printf "\377\376 binary\n"'
```

**Expected**: streamed bytes match the command's output byte-for-byte,
including the invalid byte sequence and any multi-byte character that lands
across a chunk boundary (SC-006).

Volume:

```bash
malt exec <ID> 'yes hello | head -c 100000000'
```

**Expected**: the command completes, and the daemon's memory for that session
stays bounded throughout (SC-004). Watch RSS during the run, not only after.

---

## Gate check before completion

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo build -p mash && MASH="$(pwd)/target/debug/mash.exe" cargo test -p mash --test smoosh_runner smoosh_conformance_tests -- --nocapture
```

Smoosh is a **gate** for this feature, not a formality: `mash`'s executor is
modified, and the capture-versus-stream distinction (research R2) is exactly
what command substitution and pipeline conformance tests exercise. Expected:
183 passed, 3 skipped.
