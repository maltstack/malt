# Phase 2: `mash` Sub-Project 2 — Environment (Env)

## Goal

Build the shell environment module for mash — variable storage with scope stack, special parameters, shell options, aliases, functions, traps, and session-scoped persistence. This is the foundation for expansion and execution.

## Architecture

Single `env.rs` module within the `mash` crate. The `Env` struct is the central state container threaded through all shell operations. Variables use a scope stack (push on function entry, pop on return). Special parameters ($?, $!, $$, positionals) are stored as strings. Session-scoped persistence via `EnvSnapshot` — variables survive detach/reattach and daemon restart.

## Reference

`C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\env.rs` (1,312 lines). Port the logic and data model. Rewrite with proper quality — methods instead of `pub(crate)` field access, no dropping features that could affect conformance.

---

## Module Structure

```
orix/malt/crates/mash/src/
  env.rs              # Env struct, Variable, VarValue, ShellOptions, EnvSnapshot, all methods
```

Single file. The reference was 1,312 lines — mash version should be comparable or smaller with better quality.

---

## Core Types

### Variable

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub value: VarValue,
    pub exported: bool,
    pub readonly: bool,
    pub integer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VarValue {
    String(String),
    Integer(i64, String),           // (numeric value, original string representation)
    Array(Vec<Option<String>>),     // sparse array (None = unset index)
    AssocArray(HashMap<String, String>),
}
```

### ShellOptions

All 13 flags from the reference — no dropping anything:

```rust
#[derive(Debug, Clone, Default)]
pub struct ShellOptions {
    pub errexit: bool,          // set -e
    pub nounset: bool,          // set -u
    pub pipefail: bool,         // set -o pipefail
    pub xtrace: bool,           // set -x
    pub verbose: bool,          // set -v
    pub noglob: bool,           // set -f
    pub notify: bool,           // set -b
    pub monitor: bool,          // set -m
    pub noclobber: bool,        // set -C
    pub noexec: bool,           // set -n
    pub nonlexicalctrl: bool,   // set -o nonlexicalctrl
    pub hash_cmds: bool,        // set -h
    pub nolog: bool,            // set -o nolog
}
```

Method: `flags_string() -> String` — returns POSIX `$-` string of enabled single-letter flags.

### Env

```rust
pub struct Env {
    // Variable storage — scope stack for function calls
    scopes: Vec<HashMap<String, Variable>>,
    unset_masks: Vec<HashSet<String>>,

    // Special parameters ($?, $!, $$, $0, $1...$N, $#, $@, $*)
    special: HashMap<String, String>,

    // Shell options (set -e, set -x, etc.)
    options: ShellOptions,

    // Functions — name → (source text, parsed AST)
    functions: HashMap<String, FunctionDef>,

    // Aliases — name → expansion
    aliases: HashMap<String, String>,

    // Traps — signal name → action
    traps: HashMap<String, TrapAction>,

    // Runtime state (not persisted)
    loop_control: LoopControl,
    call_stack: Vec<CallFrame>,
    call_depth: usize,
    loop_depth: usize,
    exit_requested: Option<i32>,
    is_interactive: bool,
    dir_stack: Vec<String>,
    hash_table: HashMap<String, String>,
    disabled_builtins: HashSet<String>,
    suppress_errexit: bool,
}
```

**FunctionDef** stores both source and parsed AST:
```rust
pub struct FunctionDef {
    pub source: String,                     // Original source text (for persistence)
    pub body: Spanned<Command>,             // Parsed AST (for execution)
}
```

**TrapAction:**
```rust
pub struct TrapAction {
    pub action: String,
    pub inherited: bool,
}
```

**LoopControl:**
```rust
pub enum LoopControl {
    None,
    Break(usize),
    Continue(usize),
    Return(i32),
}
```

**CallFrame:**
```rust
pub struct CallFrame {
    pub name: String,
    pub file: String,
    pub line: usize,
}
```

**EnvError:**
```rust
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("scope stack is empty")]
    EmptyScopes,
    #[error("{0}: readonly variable")]
    ReadonlyVariable(String),
}
```

---

## Public API

### Constructors

```rust
pub fn from_os() -> Self
```
Pre-populate from `std::env::vars()`. Set `$$` to current PID. Set `$0` to program name.

```rust
pub fn empty() -> Self
```
Minimal env for testing. One global scope, no variables, default options.

### Variable Access

```rust
pub fn get(&self, name: &str) -> Option<&Variable>
```
Walk scope stack top-to-bottom, respecting unset masks.

```rust
pub fn get_str(&self, name: &str) -> &str
```
Get string value. Checks special parameters first, then scopes. Returns `""` if unset.

```rust
pub fn is_set(&self, name: &str) -> bool
```

```rust
pub fn set(&mut self, name: &str, var: Variable) -> Result<(), EnvError>
```
Set in current (top) scope. Error if readonly.

```rust
pub fn set_global(&mut self, name: &str, var: Variable) -> Result<(), EnvError>
```
Set in global (bottom) scope.

```rust
pub fn unset(&mut self, name: &str) -> Result<bool, EnvError>
```
Unset variable. Error if readonly. Records in unset mask.

```rust
pub fn mark_readonly(&mut self, name: &str)
pub fn mark_exported(&mut self, name: &str)
```

### Scope Management

```rust
pub fn push_scope(&mut self)
pub fn pop_scope(&mut self) -> Result<(), EnvError>
```

### Special Parameters

```rust
pub fn set_positional_params(&mut self, command_name: &str, args: &[String])
pub fn replace_positional_args(&mut self, args: &[String])
pub fn save_positional(&self) -> HashMap<String, String>
pub fn restore_positional(&mut self, saved: HashMap<String, String>)
pub fn set_exit_code(&mut self, code: i32)
pub fn exit_code(&self) -> i32
pub fn set_last_bg_pid(&mut self, pid: u32)
```

### Options

```rust
pub fn options(&self) -> &ShellOptions
pub fn options_mut(&mut self) -> &mut ShellOptions
```

### Functions

```rust
pub fn define_function(&mut self, name: String, source: String, body: Spanned<Command>)
pub fn get_function(&self, name: &str) -> Option<&FunctionDef>
pub fn unset_function(&mut self, name: &str)
```

### Aliases

```rust
pub fn set_alias(&mut self, name: String, value: String)
pub fn get_alias(&self, name: &str) -> Option<&str>
pub fn unset_alias(&mut self, name: &str) -> bool
pub fn aliases(&self) -> &HashMap<String, String>
```

### Traps

```rust
pub fn set_trap(&mut self, signal: String, action: TrapAction)
pub fn get_trap(&self, signal: &str) -> Option<&TrapAction>
pub fn clear_trap(&mut self, signal: &str)
```

### Bulk Access

```rust
pub fn exported_vars(&self) -> HashMap<String, String>
pub fn all_variables(&self) -> HashMap<String, &Variable>
```

### Runtime State

```rust
pub fn loop_control(&self) -> &LoopControl
pub fn set_loop_control(&mut self, ctrl: LoopControl)
pub fn push_call(&mut self, frame: CallFrame)
pub fn pop_call(&mut self)
pub fn call_depth(&self) -> usize
pub fn loop_depth(&self) -> usize
pub fn set_loop_depth(&mut self, depth: usize)
pub fn request_exit(&mut self, code: i32)
pub fn exit_requested(&self) -> Option<i32>
pub fn is_interactive(&self) -> bool
pub fn dir_stack(&self) -> &[String]
pub fn push_dir(&mut self, dir: String)
pub fn pop_dir(&mut self) -> Option<String>
```

---

## Persistence

### EnvSnapshot

```rust
pub struct EnvSnapshot {
    pub variables: HashMap<String, Variable>,
    pub options: ShellOptions,
    pub aliases: HashMap<String, String>,
    pub functions: HashMap<String, String>,   // name → source text
    pub dir_stack: Vec<String>,
    pub cwd: String,
    pub traps: HashMap<String, String>,       // signal → action string
}
```

### API

```rust
impl Env {
    pub fn to_snapshot(&self) -> EnvSnapshot;
    pub fn apply_snapshot(&mut self, snapshot: &EnvSnapshot);
}
```

`to_snapshot()` flattens the scope stack — only global scope variables survive (function-local scopes are runtime-only). Functions are serialized as source text.

`apply_snapshot()` loads variables into global scope, restores options, aliases, re-parses function source texts into AST. Invalid function source is logged and skipped (not a fatal error).

Session-scoped persistence: when you `export FOO=bar` in session 3, it's stored in the session's `PersistedSession` data. On detach/reattach or daemon restart, the EnvSnapshot is restored and `FOO=bar` is back.

---

## Dependencies

```toml
# Added to existing mash Cargo.toml [dependencies]
# No new external dependencies — env.rs uses only std + existing ast types
```

The `env` module imports from `crate::ast::{Spanned, Command}` for function storage, and from `crate::parser::parse` for function source re-parsing on restore.

---

## Testing Strategy

1. **Variable scope** — set/get in global, push scope, set in child, verify isolation, pop scope, verify parent unchanged. Readonly enforcement (set → error). Unset with mask propagation across scopes.

2. **Special parameters** — set positional params, verify `$#`, `$@`, `$*` auto-update. Save/restore around simulated function call. Exit code tracking via `set_exit_code` / `exit_code`. PID storage.

3. **Options** — Set/clear individual flags, verify `flags_string()` produces correct `$-` string. All 13 flags tested.

4. **Aliases** — Define, lookup, unset. Verify `unset_alias` returns false for nonexistent.

5. **Functions** — Define with source + AST, lookup, unset. Verify source text stored for persistence.

6. **Persistence** — Create Env with variables, aliases, functions, options. Snapshot. Create fresh Env. Apply snapshot. Verify all state matches. Verify function re-parsing works.

7. **Constructors** — `from_os()` picks up OS environment, has `$$` set. `empty()` has one scope, default options.

8. **Edge cases** — Readonly variable unset (error). Empty scope pop (error). Nested scopes (3+ deep). Unset mask hides parent but not grandparent (correct POSIX behavior).
