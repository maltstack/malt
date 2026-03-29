# `mash` Executor Core — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the command execution engine — dispatch parsed AST to builtins/functions/external processes, handle pipelines via OS pipes + threads, manage redirects, wire up command substitution in the expander.

**Architecture:** Sync executor. Pipelines use `std::thread` + OS pipes. The executor calls the expander for word expansion; the expander calls `executor::capture_command` for `$(cmd)`. Builtin trait + registry with 11 flow-control builtins.

**Tech Stack:** Rust, malt-platform (process spawn, pipes), std::thread

**Spec:** `malt/specs/phase2-mash-executor.md`

**Reference:** `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\executor.rs` (7,427 lines, async) — port logic sync.

---

## File Structure

```
orix/malt/crates/mash/
  Cargo.toml              # MODIFY — add malt-platform, tempfile (dev)
  src/
    lib.rs                # MODIFY — add pub mod executor, pub mod builtins
    executor.rs           # NEW
    builtins.rs           # NEW
    expander.rs           # MODIFY — replace command sub stub
  tests/
    executor.rs           # NEW
```

---

## Tasks Overview

| Task | What | Key deliverable |
|------|------|----------------|
| 1 | Scaffold + simple external commands | ExecResult, execute(), spawn via malt-platform |
| 2 | Redirect handling | ResolvedIo, all RedirectKind variants |
| 3 | Pipeline execution | OS pipes + std::thread, pipefail, negation |
| 4 | Control flow | If/while/for/case/function/subshell/arithmetic |
| 5 | Builtins | Trait + registry + 11 flow-control builtins |
| 6 | Command sub wiring + set -e + background | Replace expander stub, errexit, $! |

Dependencies: 1 → 2 → 3; 1 → 4; 1 → 5; {2,3,4,5} → 6.

**Reference implementation:** `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\executor.rs`

**IMPORTANT for all tasks:**
- The AST uses Span (byte offsets into source text). The executor needs the source text to resolve spans. The implementer must decide how to thread source text through execution — either store it in Env, pass it alongside commands, or convert spans to owned strings at parse time. Read the existing parser and expander to understand the current pattern.
- All execution is SYNC. No async, no tokio. Use std::thread for pipelines.
- Port the logic from the reference. Rewrite with quality.
- Every `unsafe` block must have a `// SAFETY:` comment.
- Use `thiserror` for errors, `tracing` for logging.
- No `unwrap()` or `expect()` in non-test code.

---

## Task 1-6: Implementation Details

Each task follows the plan structure: implement the feature, write tests, run tests, commit.

**Test helper pattern** (used across all tasks):

```rust
use mash::parser::parse;
use mash::env::Env;
use mash::executor::{execute_list, ExecResult};

fn run(input: &str) -> (ExecResult, Env) {
    let cmds = parse(input).unwrap();
    let mut env = Env::from_os();
    // Store source text so executor can resolve spans
    // (implementation detail — see note above)
    let result = execute_list(&cmds, &mut env);
    (result, env)
}

fn run_stdout(input: &str) -> String {
    let (result, _) = run(input);
    String::from_utf8_lossy(&result.stdout).to_string()
}
```

**Test coverage per task:**

Task 1: echo hello, exit codes, sequential list, env assign
Task 2: output redirect to file, input redirect, heredoc as stdin
Task 3: 2-stage pipeline, 3-stage pipeline, negated pipeline
Task 4: if/else, for loop, while loop, case, function call, scope isolation, subshell, arithmetic, break, continue
Task 5: true, false, exit, eval, shift, return, colon
Task 6: command substitution (simple, nested, in variable), set -e (basic, suppressed in if), background $!

See the spec for the complete test list (20 tests).

---

## Verification

After all tasks:

1. `cargo test -p mash` — all tests pass
2. `cargo test --workspace` — 333+ tests, 0 failures
3. External commands: echo, cat, sort, grep
4. Pipelines: echo hello | cat | wc
5. Redirects: > file, < file, heredoc, 2>&1
6. Control flow: if/while/for/case/function
7. Command substitution: $(cmd), backtick
8. set -e with proper suppression
9. Builtins: true, false, exit, return, break, continue, eval, shift
