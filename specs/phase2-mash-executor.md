# Phase 2: `mash` Sub-Project 4 — Executor Core

## Goal

Build the command execution engine — takes parsed AST, expands words, dispatches to builtins/functions/external processes, handles pipelines, redirects, control flow, and wires up command substitution in the expander. This is where mash becomes a real shell.

## Architecture

Two modules: `executor.rs` (command dispatch, pipeline, redirects, control flow) and `builtins.rs` (registry + ~11 flow-control builtins). Sync execution — pipelines use `std::thread` + OS pipes for concurrency. The executor calls the expander for word expansion; the expander calls `executor::capture_command` for `$(cmd)`. Both are in the same crate — no circular dependency issues.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\executor.rs` (7,427 lines, async). Port the logic sync. Rewrite with quality.

---

## Module Structure

```
orix/malt/crates/mash/src/
  executor.rs           # execute(), execute_list(), ExecResult, dispatch, pipeline, redirects
  builtins.rs           # BuiltinRegistry, Builtin trait, flow-control builtins
  lib.rs                # MODIFY — add pub mod executor, pub mod builtins
  expander.rs           # MODIFY — replace command substitution stub with real execution
```

---

## Public API

```rust
/// Result of executing a command.
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Execute a single command.
pub fn execute(cmd: &Spanned<Command>, env: &mut Env) -> ExecResult;

/// Execute a list of commands sequentially.
pub fn execute_list(commands: &[Spanned<Command>], env: &mut Env) -> ExecResult;

/// Execute a command string and capture stdout. Used by expander for $(cmd).
/// Runs in a cloned Env (subshell semantics). Strips trailing newlines.
pub fn capture_command(cmd_str: &str, env: &mut Env) -> Result<String, crate::expander::ExpandError>;
```

---

## Command Dispatch

`execute()` matches on `Command` variant:

| Variant | Handler | Notes |
|---------|---------|-------|
| `Empty` | Return success | |
| `EnvAssign` | Expand values, set in env | |
| `Simple` | `execute_simple()` | Builtin → function → external |
| `Pipeline` | `execute_pipeline()` | OS pipes + std::thread |
| `List` | AND-OR chain | Respect set -e |
| `Background` | Spawn thread, set `$!` | |
| `If` | Eval condition, branch | Suppress errexit for condition |
| `While`/`Until` | Loop with re-eval | Respect break/continue |
| `For` | Expand words, iterate | |
| `ForArith` | Arithmetic loop | |
| `Case` | Expand word, pattern match | |
| `Select` | Interactive menu | |
| `FunctionDef` | Register in env | |
| `BraceGroup` | `execute_list(body)` | Same scope |
| `Subshell` | Clone env, execute | |
| `Arithmetic` | `eval_arithmetic` | Exit 0 if nonzero result |
| `Conditional` | Evaluate `[[ expr ]]` | |
| `Redirected` | Execute inner, apply redirects | |
| `Coproc`/`Time` | Stub — return error | Later sub-project |

---

## Simple Command Execution

POSIX dispatch order:

1. **Expand redirect targets** via `expand_word_nosplit`
2. **Apply per-command env assigns** (temporary, exported)
3. **Expand command name + args** via `expand_word`
4. **Resolve redirects** to files/pipes/fd-dups
5. **Dispatch:**
   - a. Builtin registry lookup → `builtin.execute(args, env)`
   - b. Function lookup → push scope, bind positionals, execute body, pop scope
   - c. External process → PATH resolve, `malt_platform::process::spawn()`
6. **Restore temp env assigns** (non-special builtins only)

### Function execution:
- Call depth limit: 50
- `env.push_scope()`, set positional params, execute body, `env.pop_scope()`
- Save/restore positionals around the call
- `LoopControl::Return(code)` exits the function

### External process execution:
- Build `SpawnConfig` with program, args, exported env vars, cwd, redirected I/O
- Stdout/stderr captured via `Io::Pipe`
- Wait for exit, build `ExecResult`

---

## Pipeline Execution

Sync with `std::thread` + OS pipes:

1. Create N-1 pipe pairs via `malt_platform::io::create_pipe()`
2. Spawn each stage in `std::thread::spawn` with its pipe ends
3. Each stage: redirect stdin from read end, stdout to write end, execute
4. Parent drops all pipe ends → stages get EOF
5. Collect exit codes via `thread.join()`
6. Pipefail: fail if any stage failed. Else: last stage's code.
7. Negation: invert final code

---

## Redirect Handling

`resolve_redirects(redirects, env) -> Result<ResolvedIo, ExecResult>`

```rust
struct ResolvedIo {
    stdin: Option<std::fs::File>,
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
}
```

| RedirectKind | Action |
|-------------|--------|
| Output | Create/truncate file (check noclobber) |
| Append | Open for append |
| Clobber | Create/truncate (ignore noclobber) |
| Input | Open for reading |
| InputOutput | Open read-write |
| HereDoc/HereDocStrip | Write body to pipe, pass read end as stdin |
| HereString | Write string to pipe, pass read end as stdin |
| DupInput/DupOutput | Duplicate fd N, or close if target is "-" |
| Both | Create file, assign to both stdout and stderr |

---

## Command Substitution Wiring

Replace the stub in `expander.rs`:

```rust
// Before (stub):
fn capture_command(_cmd: &str, _env: &mut Env) -> Result<String, ExpandError> {
    Err(ExpandError::CommandSubstitution("not available".into()))
}

// After (real):
fn capture_command(cmd: &str, env: &mut Env) -> Result<String, ExpandError> {
    crate::executor::capture_command(cmd, env)
}
```

The executor's `capture_command`:
1. Parse `cmd` via `crate::parser::parse()`
2. Clone env (subshell semantics)
3. Execute, capture stdout
4. Strip trailing newlines
5. Return captured string

---

## set -e (errexit)

After each command in a list:
- If errexit active AND exit code nonzero AND not suppressed → set `env.exit_requested`
- Suppressed for: if/while/until conditions, negated pipelines, AND-OR chains

---

## Builtin Registry

```rust
pub trait Builtin: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, args: &[String], env: &mut Env) -> ExecResult;
    fn is_special(&self) -> bool;
}

pub struct BuiltinRegistry {
    builtins: HashMap<String, Box<dyn Builtin>>,
}

impl BuiltinRegistry {
    pub fn new() -> Self; // Pre-populates with flow-control builtins
    pub fn get(&self, name: &str) -> Option<&dyn Builtin>;
    pub fn register(&mut self, builtin: Box<dyn Builtin>);
}
```

### Flow-control builtins (this sub-project):

| Builtin | Special | Behavior |
|---------|---------|----------|
| `eval` | Yes | Re-parse and execute args as shell code |
| `exec` | Yes | Replace process (or apply redirects only) |
| `source`/`.` | Yes | Read and execute file in current env |
| `exit` | Yes | Set `env.request_exit(code)` |
| `return` | Yes | Set `LoopControl::Return(code)` |
| `break` | Yes | Set `LoopControl::Break(n)` |
| `continue` | Yes | Set `LoopControl::Continue(n)` |
| `true` | No | Return ExecResult with code 0 |
| `false` | No | Return ExecResult with code 1 |
| `:` | Yes | No-op, return 0 |
| `shift` | Yes | Shift positional params by n (default 1) |

---

## Dependencies

```toml
# Added to mash Cargo.toml
malt-platform = { path = "../malt-platform" }
```

The executor needs `malt_platform::process::spawn` for external commands and `malt_platform::io::create_pipe` for pipelines/heredocs.

---

## Testing Strategy

1. **Simple external command** — `echo hello`, capture stdout, verify "hello\n"
2. **Pipeline** — `echo hello | cat`, verify output passes through
3. **Multi-stage pipeline** — `echo a b c | tr ' ' '\n' | sort`
4. **Redirects** — `echo hello > file`, read file, verify contents
5. **Input redirect** — `cat < file`, verify reads from file
6. **Heredoc** — `cat <<EOF\nhello\nEOF`, verify output
7. **If/else** — `if true; then echo yes; else echo no; fi` → "yes"
8. **For loop** — `for x in a b c; do echo $x; done` → "a\nb\nc\n"
9. **While loop** — counter with arithmetic
10. **Case** — pattern matching dispatches correctly
11. **Function def + call** — define function, call it, verify scope isolation
12. **Function positional params** — `f() { echo $1; }; f hello` → "hello"
13. **Command substitution** — `echo $(echo hello)` → "hello" (expander wiring)
14. **Nested command substitution** — `echo $(echo $(echo deep))` → "deep"
15. **set -e** — command fails → execution stops
16. **Pipefail** — middle stage fails → pipeline fails
17. **Builtins** — eval, exit code, return, break, continue, shift
18. **Env assign with command** — `FOO=bar env | grep FOO` (temp assign)
19. **Background** — `echo hello &` — verify `$!` set
20. **Negated pipeline** — `! false` → exit 0
