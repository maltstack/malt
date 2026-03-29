# `mash` Env Module — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the shell environment module — variable scope stack, special parameters, shell options, aliases, functions, traps, and session-scoped persistence via EnvSnapshot.

**Architecture:** Single `env.rs` module within the `mash` crate. `Env` is the central state container. Variables use a scope stack (push/pop for function calls). Special parameters ($?, $!, $$, positionals) stored as strings. Persistence via `EnvSnapshot` that flattens to global scope.

**Tech Stack:** Rust, std collections (HashMap, HashSet, Vec), thiserror

**Spec:** `malt/specs/phase2-mash-env.md`

**Reference:** `C:\Users\mamuk\projects\vexil-v2\vexil-shell\src\env.rs` (1,312 lines) — port logic, rewrite with quality.

---

## File Structure

```
orix/malt/crates/mash/src/
  env.rs              # NEW — Env, Variable, VarValue, ShellOptions, EnvSnapshot, all methods
  lib.rs              # MODIFY — add pub mod env
```

---

## Task 1: Core Types + Constructors

**Files:**
- Create: `orix/malt/crates/mash/src/env.rs`
- Modify: `orix/malt/crates/mash/src/lib.rs`
- Create: `orix/malt/crates/mash/tests/env.rs`

Implement Variable, VarValue, ShellOptions, EnvError, and the Env struct with constructors.

- [ ] **Step 1: Add env module to lib.rs**

Add `pub mod env;` to `orix/malt/crates/mash/src/lib.rs`.

- [ ] **Step 2: Write the core types**

Create `orix/malt/crates/mash/src/env.rs` with all types from the spec:

```rust
//! Shell environment — variable scope stack, special parameters, options, persistence.

use std::collections::{HashMap, HashSet};
use crate::ast::{Spanned, Command};

// ── Variable storage ──

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
    Integer(i64, String),
    Array(Vec<Option<String>>),
    AssocArray(HashMap<String, String>),
}

impl Variable {
    pub fn string(s: impl Into<String>) -> Self {
        Self { value: VarValue::String(s.into()), exported: false, readonly: false, integer: false }
    }

    pub fn exported_string(s: impl Into<String>) -> Self {
        Self { value: VarValue::String(s.into()), exported: true, readonly: false, integer: false }
    }

    pub fn as_str(&self) -> &str {
        match &self.value {
            VarValue::String(s) => s,
            VarValue::Integer(_, s) => s,
            VarValue::Array(_) | VarValue::AssocArray(_) => "",
        }
    }
}

// ── Shell options ──

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellOptions {
    pub errexit: bool,
    pub nounset: bool,
    pub pipefail: bool,
    pub xtrace: bool,
    pub verbose: bool,
    pub noglob: bool,
    pub notify: bool,
    pub monitor: bool,
    pub noclobber: bool,
    pub noexec: bool,
    pub nonlexicalctrl: bool,
    pub hash_cmds: bool,
    pub nolog: bool,
}

impl ShellOptions {
    pub fn flags_string(&self) -> String {
        let mut s = String::new();
        if self.errexit { s.push('e'); }
        if self.nounset { s.push('u'); }
        if self.xtrace { s.push('x'); }
        if self.verbose { s.push('v'); }
        if self.noglob { s.push('f'); }
        if self.notify { s.push('b'); }
        if self.monitor { s.push('m'); }
        if self.noclobber { s.push('C'); }
        if self.noexec { s.push('n'); }
        if self.hash_cmds { s.push('h'); }
        s
    }
}

// ── Supporting types ──

#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub source: String,
    pub body: Spanned<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrapAction {
    pub action: String,
    pub inherited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopControl {
    None,
    Break(usize),
    Continue(usize),
    Return(i32),
}

impl Default for LoopControl {
    fn default() -> Self { Self::None }
}

#[derive(Debug, Clone)]
pub struct CallFrame {
    pub name: String,
    pub file: String,
    pub line: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EnvError {
    #[error("scope stack is empty")]
    EmptyScopes,
    #[error("{0}: readonly variable")]
    ReadonlyVariable(String),
}

// ── Env struct ──

pub struct Env {
    scopes: Vec<HashMap<String, Variable>>,
    unset_masks: Vec<HashSet<String>>,
    special: HashMap<String, String>,
    options: ShellOptions,
    functions: HashMap<String, FunctionDef>,
    aliases: HashMap<String, String>,
    traps: HashMap<String, TrapAction>,
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

Implement constructors:

```rust
impl Env {
    pub fn empty() -> Self {
        let mut env = Self {
            scopes: vec![HashMap::new()],
            unset_masks: vec![HashSet::new()],
            special: HashMap::new(),
            options: ShellOptions::default(),
            functions: HashMap::new(),
            aliases: HashMap::new(),
            traps: HashMap::new(),
            loop_control: LoopControl::None,
            call_stack: Vec::new(),
            call_depth: 0,
            loop_depth: 0,
            exit_requested: None,
            is_interactive: false,
            dir_stack: Vec::new(),
            hash_table: HashMap::new(),
            disabled_builtins: HashSet::new(),
            suppress_errexit: false,
        };
        env.special.insert("$".to_string(), std::process::id().to_string());
        env.special.insert("?".to_string(), "0".to_string());
        env
    }

    pub fn from_os() -> Self {
        let mut env = Self::empty();
        for (key, value) in std::env::vars() {
            env.scopes[0].insert(key, Variable::exported_string(value));
        }
        if let Ok(cwd) = std::env::current_dir() {
            env.special.insert("PWD".to_string(), cwd.to_string_lossy().to_string());
        }
        env
    }
}
```

- [ ] **Step 3: Write constructor tests**

Create `orix/malt/crates/mash/tests/env.rs`:

```rust
use mash::env::*;

#[test]
fn empty_env_has_defaults() {
    let env = Env::empty();
    assert_eq!(env.exit_code(), 0);
    assert!(!env.is_interactive());
    assert_eq!(env.options().errexit, false);
}

#[test]
fn empty_env_has_pid() {
    let env = Env::empty();
    let pid = env.get_str("$");
    assert!(!pid.is_empty());
    assert!(pid.parse::<u32>().is_ok());
}

#[test]
fn from_os_has_path() {
    let env = Env::from_os();
    // PATH should exist on all platforms
    assert!(env.get("PATH").is_some() || env.get("Path").is_some());
}

#[test]
fn from_os_vars_are_exported() {
    let env = Env::from_os();
    if let Some(var) = env.get("PATH").or(env.get("Path")) {
        assert!(var.exported);
    }
}
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cd orix/malt && cargo test -p mash --test env`

- [ ] **Step 5: Commit**

```bash
cd orix/malt
git add crates/mash/src/env.rs crates/mash/src/lib.rs crates/mash/tests/env.rs
git commit -m "feat(mash): env core types — Variable, ShellOptions, Env constructors"
```

---

## Task 2: Variable Access + Scope Stack

**Files:**
- Modify: `orix/malt/crates/mash/src/env.rs`
- Modify: `orix/malt/crates/mash/tests/env.rs`

Implement variable get/set/unset with scope stack traversal and unset masks.

- [ ] **Step 1: Implement variable access methods**

Add to `impl Env`:

```rust
pub fn get(&self, name: &str) -> Option<&Variable> {
    // Check special parameters first
    // Then walk scopes top-to-bottom, respecting unset masks
    for (i, scope) in self.scopes.iter().enumerate().rev() {
        if self.unset_masks[i].contains(name) {
            return None; // Explicitly unset in this scope
        }
        if let Some(var) = scope.get(name) {
            return Some(var);
        }
    }
    None
}

pub fn get_str(&self, name: &str) -> &str {
    // Special parameters first
    if let Some(val) = self.special.get(name) {
        return val;
    }
    // Then scoped variables
    self.get(name).map(|v| v.as_str()).unwrap_or("")
}

pub fn is_set(&self, name: &str) -> bool {
    self.special.contains_key(name) || self.get(name).is_some()
}

pub fn set(&mut self, name: &str, var: Variable) -> Result<(), EnvError> {
    // Check readonly in all scopes
    if let Some(existing) = self.get(name) {
        if existing.readonly {
            return Err(EnvError::ReadonlyVariable(name.to_string()));
        }
    }
    let top = self.scopes.last_mut().unwrap();
    top.insert(name.to_string(), var);
    // Clear unset mask if present
    self.unset_masks.last_mut().unwrap().remove(name);
    Ok(())
}

pub fn set_global(&mut self, name: &str, var: Variable) -> Result<(), EnvError> {
    if let Some(existing) = self.scopes[0].get(name) {
        if existing.readonly {
            return Err(EnvError::ReadonlyVariable(name.to_string()));
        }
    }
    self.scopes[0].insert(name.to_string(), var);
    self.unset_masks[0].remove(name);
    Ok(())
}

pub fn unset(&mut self, name: &str) -> Result<bool, EnvError> {
    if let Some(existing) = self.get(name) {
        if existing.readonly {
            return Err(EnvError::ReadonlyVariable(name.to_string()));
        }
    }
    let top_idx = self.scopes.len() - 1;
    self.scopes[top_idx].remove(name);
    self.unset_masks[top_idx].insert(name.to_string());
    Ok(true)
}

pub fn mark_readonly(&mut self, name: &str) {
    if let Some(scope) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
        if let Some(var) = scope.get_mut(name) {
            var.readonly = true;
        }
    }
}

pub fn mark_exported(&mut self, name: &str) {
    if let Some(scope) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
        if let Some(var) = scope.get_mut(name) {
            var.exported = true;
        }
    }
}
```

- [ ] **Step 2: Implement scope management**

```rust
pub fn push_scope(&mut self) {
    self.scopes.push(HashMap::new());
    self.unset_masks.push(HashSet::new());
}

pub fn pop_scope(&mut self) -> Result<(), EnvError> {
    if self.scopes.len() <= 1 {
        return Err(EnvError::EmptyScopes);
    }
    self.scopes.pop();
    self.unset_masks.pop();
    Ok(())
}
```

- [ ] **Step 3: Implement bulk access**

```rust
pub fn exported_vars(&self) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for scope in &self.scopes {
        for (name, var) in scope {
            if var.exported {
                result.insert(name.clone(), var.as_str().to_string());
            }
        }
    }
    result
}

pub fn all_variables(&self) -> HashMap<String, &Variable> {
    let mut result = HashMap::new();
    for scope in &self.scopes {
        for (name, var) in scope {
            result.entry(name.clone()).or_insert(var);
        }
    }
    result
}
```

- [ ] **Step 4: Write variable and scope tests**

Add to `tests/env.rs`:

```rust
#[test]
fn set_and_get_variable() {
    let mut env = Env::empty();
    env.set("FOO", Variable::string("bar")).unwrap();
    assert_eq!(env.get_str("FOO"), "bar");
}

#[test]
fn get_unset_returns_empty() {
    let env = Env::empty();
    assert_eq!(env.get_str("NONEXISTENT"), "");
    assert!(!env.is_set("NONEXISTENT"));
}

#[test]
fn scope_isolation() {
    let mut env = Env::empty();
    env.set("X", Variable::string("global")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("local")).unwrap();
    assert_eq!(env.get_str("X"), "local");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "global");
}

#[test]
fn scope_child_sees_parent() {
    let mut env = Env::empty();
    env.set("X", Variable::string("parent")).unwrap();
    env.push_scope();
    assert_eq!(env.get_str("X"), "parent");
    env.pop_scope().unwrap();
}

#[test]
fn unset_masks_parent() {
    let mut env = Env::empty();
    env.set("X", Variable::string("parent")).unwrap();
    env.push_scope();
    env.unset("X").unwrap();
    assert!(!env.is_set("X"));
    assert_eq!(env.get_str("X"), "");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "parent"); // Parent unaffected
}

#[test]
fn readonly_prevents_set() {
    let mut env = Env::empty();
    env.set("X", Variable::string("val")).unwrap();
    env.mark_readonly("X");
    assert!(env.set("X", Variable::string("new")).is_err());
}

#[test]
fn readonly_prevents_unset() {
    let mut env = Env::empty();
    env.set("X", Variable::string("val")).unwrap();
    env.mark_readonly("X");
    assert!(env.unset("X").is_err());
}

#[test]
fn pop_global_scope_fails() {
    let mut env = Env::empty();
    assert!(env.pop_scope().is_err());
}

#[test]
fn nested_scopes_three_deep() {
    let mut env = Env::empty();
    env.set("X", Variable::string("0")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("1")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("2")).unwrap();
    assert_eq!(env.get_str("X"), "2");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "1");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "0");
}

#[test]
fn exported_vars_only_exported() {
    let mut env = Env::empty();
    env.set("A", Variable::string("local")).unwrap();
    env.set("B", Variable::exported_string("exported")).unwrap();
    let exported = env.exported_vars();
    assert!(!exported.contains_key("A"));
    assert_eq!(exported.get("B").unwrap(), "exported");
}

#[test]
fn set_global_bypasses_scope() {
    let mut env = Env::empty();
    env.push_scope();
    env.set_global("X", Variable::string("global")).unwrap();
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "global");
}
```

- [ ] **Step 5: Run tests and commit**

Run: `cd orix/malt && cargo test -p mash --test env`

```bash
cd orix/malt
git add crates/mash/src/env.rs crates/mash/tests/env.rs
git commit -m "feat(mash): env variable access — get/set/unset with scope stack, readonly, export"
```

---

## Task 3: Special Parameters + Options

**Files:**
- Modify: `orix/malt/crates/mash/src/env.rs`
- Modify: `orix/malt/crates/mash/tests/env.rs`

- [ ] **Step 1: Implement special parameter methods**

```rust
pub fn set_positional_params(&mut self, command_name: &str, args: &[String]) {
    // Clear old positionals
    self.special.retain(|k, _| {
        k.parse::<usize>().is_err() && k != "#" && k != "@" && k != "*"
    });
    self.special.insert("0".to_string(), command_name.to_string());
    for (i, arg) in args.iter().enumerate() {
        self.special.insert((i + 1).to_string(), arg.clone());
    }
    self.special.insert("#".to_string(), args.len().to_string());
    self.special.insert("@".to_string(), args.join(" "));
    self.special.insert("*".to_string(), args.join(" "));
}

pub fn replace_positional_args(&mut self, args: &[String]) {
    // Keep $0, replace $1...$N
    self.special.retain(|k, _| k.parse::<usize>().map_or(true, |n| n == 0));
    for (i, arg) in args.iter().enumerate() {
        self.special.insert((i + 1).to_string(), arg.clone());
    }
    self.special.insert("#".to_string(), args.len().to_string());
    self.special.insert("@".to_string(), args.join(" "));
    self.special.insert("*".to_string(), args.join(" "));
}

pub fn save_positional(&self) -> HashMap<String, String> {
    self.special.iter()
        .filter(|(k, _)| k.parse::<usize>().is_ok() || ["#", "@", "*"].contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

pub fn restore_positional(&mut self, saved: HashMap<String, String>) {
    self.special.retain(|k, _| {
        k.parse::<usize>().is_err() && k != "#" && k != "@" && k != "*"
    });
    self.special.extend(saved);
}

pub fn set_exit_code(&mut self, code: i32) {
    self.special.insert("?".to_string(), code.to_string());
}

pub fn exit_code(&self) -> i32 {
    self.special.get("?").and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn set_last_bg_pid(&mut self, pid: u32) {
    self.special.insert("!".to_string(), pid.to_string());
}
```

- [ ] **Step 2: Implement options accessors and runtime state**

```rust
pub fn options(&self) -> &ShellOptions { &self.options }
pub fn options_mut(&mut self) -> &mut ShellOptions { &mut self.options }
pub fn is_interactive(&self) -> bool { self.is_interactive }
pub fn set_interactive(&mut self, v: bool) { self.is_interactive = v; }
pub fn loop_control(&self) -> &LoopControl { &self.loop_control }
pub fn set_loop_control(&mut self, ctrl: LoopControl) { self.loop_control = ctrl; }
pub fn call_depth(&self) -> usize { self.call_depth }
pub fn loop_depth(&self) -> usize { self.loop_depth }
pub fn set_loop_depth(&mut self, depth: usize) { self.loop_depth = depth; }
pub fn request_exit(&mut self, code: i32) { self.exit_requested = Some(code); }
pub fn exit_requested(&self) -> Option<i32> { self.exit_requested }
pub fn push_call(&mut self, frame: CallFrame) {
    self.call_stack.push(frame);
    self.call_depth += 1;
}
pub fn pop_call(&mut self) {
    self.call_stack.pop();
    self.call_depth = self.call_depth.saturating_sub(1);
}
pub fn dir_stack(&self) -> &[String] { &self.dir_stack }
pub fn push_dir(&mut self, dir: String) { self.dir_stack.push(dir); }
pub fn pop_dir(&mut self) -> Option<String> { self.dir_stack.pop() }
```

- [ ] **Step 3: Write tests**

```rust
#[test]
fn positional_params() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b".into(), "c".into()]);
    assert_eq!(env.get_str("0"), "mash");
    assert_eq!(env.get_str("1"), "a");
    assert_eq!(env.get_str("2"), "b");
    assert_eq!(env.get_str("3"), "c");
    assert_eq!(env.get_str("#"), "3");
}

#[test]
fn replace_positional_preserves_zero() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["old".into()]);
    env.replace_positional_args(&["new1".into(), "new2".into()]);
    assert_eq!(env.get_str("0"), "mash"); // preserved
    assert_eq!(env.get_str("1"), "new1");
    assert_eq!(env.get_str("2"), "new2");
    assert_eq!(env.get_str("#"), "2");
}

#[test]
fn save_restore_positional() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b".into()]);
    let saved = env.save_positional();
    env.replace_positional_args(&["x".into()]);
    assert_eq!(env.get_str("1"), "x");
    env.restore_positional(saved);
    assert_eq!(env.get_str("1"), "a");
    assert_eq!(env.get_str("#"), "2");
}

#[test]
fn exit_code_tracking() {
    let mut env = Env::empty();
    assert_eq!(env.exit_code(), 0);
    env.set_exit_code(42);
    assert_eq!(env.exit_code(), 42);
    assert_eq!(env.get_str("?"), "42");
}

#[test]
fn bg_pid_tracking() {
    let mut env = Env::empty();
    env.set_last_bg_pid(12345);
    assert_eq!(env.get_str("!"), "12345");
}

#[test]
fn options_flags_string() {
    let mut env = Env::empty();
    env.options_mut().errexit = true;
    env.options_mut().xtrace = true;
    let flags = env.options().flags_string();
    assert!(flags.contains('e'));
    assert!(flags.contains('x'));
    assert!(!flags.contains('u'));
}

#[test]
fn loop_control_default() {
    let env = Env::empty();
    assert_eq!(*env.loop_control(), LoopControl::None);
}

#[test]
fn dir_stack_push_pop() {
    let mut env = Env::empty();
    env.push_dir("/home".to_string());
    env.push_dir("/tmp".to_string());
    assert_eq!(env.dir_stack().len(), 2);
    assert_eq!(env.pop_dir(), Some("/tmp".to_string()));
    assert_eq!(env.dir_stack().len(), 1);
}

#[test]
fn call_depth_tracking() {
    let mut env = Env::empty();
    assert_eq!(env.call_depth(), 0);
    env.push_call(CallFrame { name: "f".into(), file: "test".into(), line: 1 });
    assert_eq!(env.call_depth(), 1);
    env.pop_call();
    assert_eq!(env.call_depth(), 0);
}
```

- [ ] **Step 4: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/env.rs crates/mash/tests/env.rs
git commit -m "feat(mash): env special parameters, options, runtime state"
```

---

## Task 4: Functions, Aliases, Traps

**Files:**
- Modify: `orix/malt/crates/mash/src/env.rs`
- Modify: `orix/malt/crates/mash/tests/env.rs`

- [ ] **Step 1: Implement function, alias, trap methods**

```rust
// Functions
pub fn define_function(&mut self, name: String, source: String, body: Spanned<Command>) {
    self.functions.insert(name, FunctionDef { source, body });
}
pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
    self.functions.get(name)
}
pub fn unset_function(&mut self, name: &str) {
    self.functions.remove(name);
}

// Aliases
pub fn set_alias(&mut self, name: String, value: String) {
    self.aliases.insert(name, value);
}
pub fn get_alias(&self, name: &str) -> Option<&str> {
    self.aliases.get(name).map(|s| s.as_str())
}
pub fn unset_alias(&mut self, name: &str) -> bool {
    self.aliases.remove(name).is_some()
}
pub fn aliases(&self) -> &HashMap<String, String> {
    &self.aliases
}

// Traps
pub fn set_trap(&mut self, signal: String, action: TrapAction) {
    self.traps.insert(signal, action);
}
pub fn get_trap(&self, signal: &str) -> Option<&TrapAction> {
    self.traps.get(signal)
}
pub fn clear_trap(&mut self, signal: &str) {
    self.traps.remove(signal);
}
```

- [ ] **Step 2: Write tests**

```rust
#[test]
fn alias_set_get_unset() {
    let mut env = Env::empty();
    env.set_alias("ll".into(), "ls -la".into());
    assert_eq!(env.get_alias("ll"), Some("ls -la"));
    assert!(env.unset_alias("ll"));
    assert_eq!(env.get_alias("ll"), None);
    assert!(!env.unset_alias("ll")); // already removed
}

#[test]
fn function_define_get_unset() {
    let mut env = Env::empty();
    let body = crate::parser::parse("echo hello").unwrap().remove(0);
    env.define_function("greet".into(), "echo hello".into(), body);
    assert!(env.get_function("greet").is_some());
    assert_eq!(env.get_function("greet").unwrap().source, "echo hello");
    env.unset_function("greet");
    assert!(env.get_function("greet").is_none());
}

#[test]
fn trap_set_get_clear() {
    let mut env = Env::empty();
    env.set_trap("INT".into(), TrapAction { action: "echo caught".into(), inherited: false });
    assert!(env.get_trap("INT").is_some());
    assert_eq!(env.get_trap("INT").unwrap().action, "echo caught");
    env.clear_trap("INT");
    assert!(env.get_trap("INT").is_none());
}
```

- [ ] **Step 3: Run tests and commit**

```bash
cd orix/malt
git add crates/mash/src/env.rs crates/mash/tests/env.rs
git commit -m "feat(mash): env functions, aliases, traps"
```

---

## Task 5: Persistence (EnvSnapshot)

**Files:**
- Modify: `orix/malt/crates/mash/src/env.rs`
- Modify: `orix/malt/crates/mash/tests/env.rs`

- [ ] **Step 1: Define EnvSnapshot type**

```rust
#[derive(Debug, Clone)]
pub struct EnvSnapshot {
    pub variables: HashMap<String, Variable>,
    pub options: ShellOptions,
    pub aliases: HashMap<String, String>,
    pub functions: HashMap<String, String>, // name → source text
    pub dir_stack: Vec<String>,
    pub cwd: String,
    pub traps: HashMap<String, String>, // signal → action string
}
```

- [ ] **Step 2: Implement to_snapshot and apply_snapshot**

```rust
impl Env {
    pub fn to_snapshot(&self) -> EnvSnapshot {
        // Flatten scope stack — only global scope variables persist
        let variables = self.scopes[0].clone();
        let functions = self.functions.iter()
            .map(|(name, def)| (name.clone(), def.source.clone()))
            .collect();
        let traps = self.traps.iter()
            .map(|(sig, trap)| (sig.clone(), trap.action.clone()))
            .collect();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        EnvSnapshot {
            variables,
            options: self.options.clone(),
            aliases: self.aliases.clone(),
            functions,
            dir_stack: self.dir_stack.clone(),
            cwd,
            traps,
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &EnvSnapshot) {
        // Load variables into global scope
        for (name, var) in &snapshot.variables {
            self.scopes[0].insert(name.clone(), var.clone());
        }
        self.options = snapshot.options.clone();
        self.aliases = snapshot.aliases.clone();
        self.dir_stack = snapshot.dir_stack.clone();

        // Re-parse function source texts
        for (name, source) in &snapshot.functions {
            match crate::parser::parse(source) {
                Ok(mut cmds) if !cmds.is_empty() => {
                    let body = cmds.remove(0);
                    self.functions.insert(name.clone(), FunctionDef {
                        source: source.clone(),
                        body,
                    });
                }
                _ => {
                    // Log warning but don't fail — invalid function source is non-fatal
                    tracing::warn!("failed to re-parse function '{}' from snapshot", name);
                }
            }
        }

        // Restore traps
        for (signal, action) in &snapshot.traps {
            self.traps.insert(signal.clone(), TrapAction {
                action: action.clone(),
                inherited: false,
            });
        }
    }
}
```

- [ ] **Step 3: Add tracing dependency**

Add `tracing = "0.1"` to `orix/malt/crates/mash/Cargo.toml` `[dependencies]`.

- [ ] **Step 4: Write persistence tests**

```rust
#[test]
fn snapshot_roundtrip_variables() {
    let mut env = Env::empty();
    env.set("FOO", Variable::exported_string("bar")).unwrap();
    env.set("BAZ", Variable::string("qux")).unwrap();
    env.options_mut().errexit = true;
    env.set_alias("ll".into(), "ls -la".into());

    let snapshot = env.to_snapshot();

    let mut restored = Env::empty();
    restored.apply_snapshot(&snapshot);

    assert_eq!(restored.get_str("FOO"), "bar");
    assert_eq!(restored.get_str("BAZ"), "qux");
    assert!(restored.get("FOO").unwrap().exported);
    assert!(restored.options().errexit);
    assert_eq!(restored.get_alias("ll"), Some("ls -la"));
}

#[test]
fn snapshot_roundtrip_functions() {
    let mut env = Env::empty();
    let body = crate::parser::parse("echo hello").unwrap().remove(0);
    env.define_function("greet".into(), "echo hello".into(), body);

    let snapshot = env.to_snapshot();
    assert_eq!(snapshot.functions.get("greet").unwrap(), "echo hello");

    let mut restored = Env::empty();
    restored.apply_snapshot(&snapshot);
    assert!(restored.get_function("greet").is_some());
}

#[test]
fn snapshot_only_global_scope() {
    let mut env = Env::empty();
    env.set("GLOBAL", Variable::string("yes")).unwrap();
    env.push_scope();
    env.set("LOCAL", Variable::string("no")).unwrap();

    let snapshot = env.to_snapshot();

    // Only global scope variables in snapshot
    assert!(snapshot.variables.contains_key("GLOBAL"));
    assert!(!snapshot.variables.contains_key("LOCAL"));
}

#[test]
fn snapshot_traps_roundtrip() {
    let mut env = Env::empty();
    env.set_trap("EXIT".into(), TrapAction { action: "echo bye".into(), inherited: false });

    let snapshot = env.to_snapshot();
    let mut restored = Env::empty();
    restored.apply_snapshot(&snapshot);

    assert_eq!(restored.get_trap("EXIT").unwrap().action, "echo bye");
}
```

- [ ] **Step 5: Run full test suite and commit**

Run: `cd orix/malt && cargo test -p mash`
Expected: All lexer + parser + env tests pass.

```bash
cd orix/malt
git add crates/mash/Cargo.toml crates/mash/src/env.rs crates/mash/tests/env.rs
git commit -m "feat(mash): env persistence — EnvSnapshot with session-scoped variable survival"
```

---

## Verification

After all tasks:

1. `cargo test -p mash` — all tests pass (lexer 78 + parser 91 + env new)
2. `cargo test --workspace` — 235+ tests, 0 failures
3. `cargo clippy -p mash -- -D warnings` — clean
4. Scope stack: push/pop/isolation tested with 3+ depth
5. Special params: positional save/restore, exit code, bg PID
6. Persistence: snapshot roundtrip preserves variables, options, aliases, functions, traps
7. Readonly: prevents set and unset, returns proper error
