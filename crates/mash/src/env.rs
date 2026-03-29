//! Shell environment — variable scope stack, special parameters, options, persistence.

use std::collections::{HashMap, HashSet};
use crate::ast::{Spanned, Command};
use crate::parser;

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
        if self.errexit   { s.push('e'); }
        if self.nounset   { s.push('u'); }
        if self.xtrace    { s.push('x'); }
        if self.verbose   { s.push('v'); }
        if self.noglob    { s.push('f'); }
        if self.notify    { s.push('b'); }
        if self.monitor   { s.push('m'); }
        if self.noclobber { s.push('C'); }
        if self.noexec    { s.push('n'); }
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

    // ── Variable access ──

    pub fn get(&self, name: &str) -> Option<&Variable> {
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            if self.unset_masks[i].contains(name) {
                return None;
            }
            if let Some(var) = scope.get(name) {
                return Some(var);
            }
        }
        None
    }

    pub fn get_str(&self, name: &str) -> &str {
        if let Some(val) = self.special.get(name) {
            return val;
        }
        self.get(name).map(|v| v.as_str()).unwrap_or("")
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.special.contains_key(name) || self.get(name).is_some()
    }

    pub fn set(&mut self, name: &str, var: Variable) -> Result<(), EnvError> {
        if let Some(existing) = self.get(name) {
            if existing.readonly {
                return Err(EnvError::ReadonlyVariable(name.to_string()));
            }
        }
        let top = self.scopes.last_mut().unwrap();
        top.insert(name.to_string(), var);
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

    // ── Scope management ──

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

    // ── Bulk access ──

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

    // ── Special parameters ──

    pub fn set_positional_params(&mut self, command_name: &str, args: &[String]) {
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

    // ── Options + runtime state ──

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

    // ── Functions ──

    pub fn define_function(&mut self, name: String, source: String, body: Spanned<Command>) {
        self.functions.insert(name, FunctionDef { source, body });
    }

    pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.get(name)
    }

    pub fn unset_function(&mut self, name: &str) {
        self.functions.remove(name);
    }

    // ── Aliases ──

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

    // ── Traps ──

    pub fn set_trap(&mut self, signal: String, action: TrapAction) {
        self.traps.insert(signal, action);
    }

    pub fn get_trap(&self, signal: &str) -> Option<&TrapAction> {
        self.traps.get(signal)
    }

    pub fn clear_trap(&mut self, signal: &str) {
        self.traps.remove(signal);
    }

    // ── Persistence ──

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
            match parser::parse(source) {
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

// ── EnvSnapshot ──

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
