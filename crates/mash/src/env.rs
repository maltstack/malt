//! Shell environment — variable scope stack, special parameters, options, persistence.

use crate::ast::{Command, Spanned};
use crate::parser;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::sync::{Arc, Mutex, MutexGuard};

const MASH_FD_ALIASES_ENV: &str = "MASH_FD_ALIASES";

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
        Self {
            value: VarValue::String(s.into()),
            exported: false,
            readonly: false,
            integer: false,
        }
    }

    pub fn exported_string(s: impl Into<String>) -> Self {
        Self {
            value: VarValue::String(s.into()),
            exported: true,
            readonly: false,
            integer: false,
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub nonlexicalctrl: bool, // Default: false (POSIX default - lexical scoping)
    pub hash_cmds: bool,
    pub nolog: bool,
    pub sourcepath: bool, // Search PATH for source command
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            errexit: false,
            nounset: false,
            pipefail: false,
            xtrace: false,
            verbose: false,
            noglob: false,
            notify: false,
            monitor: false,
            noclobber: false,
            noexec: false,
            nonlexicalctrl: false, // POSIX default: break/continue are scoped to function (lexical)
            hash_cmds: true,
            nolog: false,
            sourcepath: true, // POSIX: search PATH for source command
        }
    }
}

impl ShellOptions {
    pub fn flags_string(&self) -> String {
        let mut s = String::new();
        if self.errexit {
            s.push('e');
        }
        if self.nounset {
            s.push('u');
        }
        if self.xtrace {
            s.push('x');
        }
        if self.verbose {
            s.push('v');
        }
        if self.noglob {
            s.push('f');
        }
        if self.notify {
            s.push('b');
        }
        if self.monitor {
            s.push('m');
        }
        if self.noclobber {
            s.push('C');
        }
        if self.noexec {
            s.push('n');
        }
        if self.hash_cmds {
            s.push('h');
        }
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
pub enum JobStatus {
    Running,
    Done,
    Signaled(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobEntry {
    pub job_id: u32,
    pub pid: u32,
    pub command: String,
    pub status: JobStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopControl {
    None,
    Break(usize),
    Continue(usize),
    Return(i32),
}

impl Default for LoopControl {
    fn default() -> Self {
        Self::None
    }
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

#[derive(Clone)]
pub struct Env {
    scopes: Vec<HashMap<String, Variable>>,
    unset_masks: Vec<HashSet<String>>,
    /// Track which variables are local in each scope (for proper scope cleanup).
    local_vars: Vec<HashSet<String>>,
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
    history: Arc<Mutex<Vec<String>>>,
    jobs: Arc<Mutex<Vec<JobEntry>>>,
    /// Opaque isolation context token passed through from daemon.
    /// MASH does not interpret this; it's passed to platform spawn traits.
    isolation_context: Option<malt_platform::isolation::IsolationContext>,
    fd_registry: malt_platform::vfs::SharedFdRegistry,
    fd_aliases: Arc<Mutex<HashMap<u32, u32>>>,
}

impl Env {
    pub fn empty() -> Self {
        let mut env = Self {
            scopes: vec![HashMap::new()],
            unset_masks: vec![HashSet::new()],
            local_vars: vec![HashSet::new()],
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
            history: Arc::new(Mutex::new(Vec::new())),
            jobs: Arc::new(Mutex::new(Vec::new())),
            isolation_context: None,
            fd_registry: malt_platform::vfs::SharedFdRegistry::new(),
            fd_aliases: Arc::new(Mutex::new(HashMap::new())),
        };
        env.special
            .insert("$".to_string(), std::process::id().to_string());
        env.special.insert("?".to_string(), "0".to_string());
        env
    }

    pub fn from_os() -> Self {
        let mut env = Self::empty();
        for (key, value) in std::env::vars() {
            env.scopes[0].insert(key, Variable::exported_string(value));
        }
        if let Ok(alias_spec) = std::env::var(MASH_FD_ALIASES_ENV) {
            for entry in alias_spec.split(',').filter(|entry| !entry.is_empty()) {
                if let Some((fd_text, target_text)) = entry.split_once(':') {
                    if let (Ok(fd), Ok(target_fd)) =
                        (fd_text.parse::<u32>(), target_text.parse::<u32>())
                    {
                        env.register_fd_alias(fd, target_fd);
                    }
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            let pwd_str = {
                let s = cwd.to_string_lossy().to_string();
                #[cfg(windows)]
                {
                    s.replace('\\', "/")
                }
                #[cfg(not(windows))]
                {
                    s
                }
            };
            // PWD is a regular exported variable, not a special parameter.
            // Store it in scopes so `set()` / `get()` work correctly for cd.
            env.scopes[0].insert("PWD".to_string(), Variable::exported_string(pwd_str));
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
        let top = self.scopes.last_mut().ok_or(EnvError::EmptyScopes)?;
        top.insert(name.to_string(), var);
        self.unset_masks
            .last_mut()
            .ok_or(EnvError::EmptyScopes)?
            .remove(name);
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

    /// Mark a variable as local to the current scope.
    /// This ensures the variable is removed when the scope is popped.
    pub fn mark_local(&mut self, name: &str) {
        if let Some(locals) = self.local_vars.last_mut() {
            locals.insert(name.to_string());
        }
    }

    /// Check if a variable is local to the current scope.
    pub fn is_local(&self, name: &str) -> bool {
        self.local_vars
            .last()
            .map(|locals| locals.contains(name))
            .unwrap_or(false)
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

    pub fn mark_unexported(&mut self, name: &str) {
        if let Some(scope) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
            if let Some(var) = scope.get_mut(name) {
                var.exported = false;
            }
        }
    }

    pub fn readonly_vars(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for scope in &self.scopes {
            for (name, var) in scope {
                if var.readonly {
                    result.insert(name.clone(), var.as_str().to_string());
                }
            }
        }
        result
    }

    // ── Scope management ──

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.unset_masks.push(HashSet::new());
        self.local_vars.push(HashSet::new());
    }

    pub fn pop_scope(&mut self) -> Result<(), EnvError> {
        if self.scopes.len() <= 1 {
            return Err(EnvError::EmptyScopes);
        }
        self.scopes.pop();
        self.unset_masks.pop();
        self.local_vars.pop();
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
        self.special
            .retain(|k, _| k.parse::<usize>().is_err() && k != "#" && k != "@" && k != "*");
        self.special
            .insert("0".to_string(), command_name.to_string());
        for (i, arg) in args.iter().enumerate() {
            self.special.insert((i + 1).to_string(), arg.clone());
        }
        self.special.insert("#".to_string(), args.len().to_string());
        self.special.insert("@".to_string(), args.join(" "));
        self.special.insert("*".to_string(), args.join(" "));
    }

    pub fn replace_positional_args(&mut self, args: &[String]) {
        self.special
            .retain(|k, _| k.parse::<usize>().map_or(true, |n| n == 0));
        for (i, arg) in args.iter().enumerate() {
            self.special.insert((i + 1).to_string(), arg.clone());
        }
        self.special.insert("#".to_string(), args.len().to_string());
        self.special.insert("@".to_string(), args.join(" "));
        self.special.insert("*".to_string(), args.join(" "));
    }

    pub fn save_positional(&self) -> HashMap<String, String> {
        self.special
            .iter()
            .filter(|(k, _)| k.parse::<usize>().is_ok() || ["#", "@", "*"].contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn restore_positional(&mut self, saved: HashMap<String, String>) {
        self.special
            .retain(|k, _| k.parse::<usize>().is_err() && k != "#" && k != "@" && k != "*");
        self.special.extend(saved);
    }

    pub fn set_exit_code(&mut self, code: i32) {
        self.special.insert("?".to_string(), code.to_string());
    }

    pub fn exit_code(&self) -> i32 {
        self.special
            .get("?")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    pub fn set_last_bg_pid(&mut self, pid: u32) {
        self.special.insert("!".to_string(), pid.to_string());
    }

    // ── Options + runtime state ──

    pub fn options(&self) -> &ShellOptions {
        &self.options
    }
    pub fn options_mut(&mut self) -> &mut ShellOptions {
        &mut self.options
    }
    pub fn is_interactive(&self) -> bool {
        self.is_interactive
    }
    pub fn set_interactive(&mut self, v: bool) {
        self.is_interactive = v;
    }
    pub fn loop_control(&self) -> &LoopControl {
        &self.loop_control
    }
    pub fn set_loop_control(&mut self, ctrl: LoopControl) {
        self.loop_control = ctrl;
    }
    pub fn suppress_errexit(&self) -> bool {
        self.suppress_errexit
    }
    pub fn set_suppress_errexit(&mut self, v: bool) {
        self.suppress_errexit = v;
    }
    pub fn call_depth(&self) -> usize {
        self.call_depth
    }
    pub fn loop_depth(&self) -> usize {
        self.loop_depth
    }
    pub fn set_loop_depth(&mut self, depth: usize) {
        self.loop_depth = depth;
    }
    pub fn request_exit(&mut self, code: i32) {
        self.exit_requested = Some(code);
    }
    pub fn set_exit_requested(&mut self, code: Option<i32>) {
        self.exit_requested = code;
    }
    pub fn exit_requested(&self) -> Option<i32> {
        self.exit_requested
    }

    pub fn set_option_nonlexicalctrl(&mut self, value: bool) {
        self.options.nonlexicalctrl = value;
    }

    pub fn push_call(&mut self, frame: CallFrame) {
        self.call_stack.push(frame);
        self.call_depth += 1;
    }

    pub fn pop_call(&mut self) {
        self.call_stack.pop();
        self.call_depth = self.call_depth.saturating_sub(1);
    }

    pub fn dir_stack(&self) -> &[String] {
        &self.dir_stack
    }
    pub fn push_dir(&mut self, dir: String) {
        self.dir_stack.push(dir);
    }
    pub fn pop_dir(&mut self) -> Option<String> {
        self.dir_stack.pop()
    }

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

    pub fn clear_aliases(&mut self) {
        self.aliases.clear();
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

    pub fn traps(&self) -> &HashMap<String, TrapAction> {
        &self.traps
    }

    pub fn clear_noninherited_traps(&mut self) {
        self.traps.retain(|_, trap| trap.inherited);
    }

    // ── History ──

    pub fn push_history_entry(&self, entry: String) {
        let entry = entry.trim().to_string();
        if entry.is_empty() {
            return;
        }
        self.history_lock().push(entry);
    }

    pub fn clear_history(&self) {
        self.history_lock().clear();
    }

    pub fn history_entries(&self) -> Vec<String> {
        self.history_lock().clone()
    }

    // ── Jobs ──

    pub fn register_job(&self, job_id: u32, command: String) {
        let mut jobs = self.jobs_lock();
        jobs.push(JobEntry {
            job_id,
            pid: job_id,
            command,
            status: JobStatus::Running,
        });
    }

    pub fn mark_job_done(&self, job_id: u32) {
        if let Some(job) = self.jobs_lock().iter_mut().find(|job| job.job_id == job_id) {
            job.status = JobStatus::Done;
        }
    }

    pub fn signal_job(&self, pid: u32, signal: String) -> bool {
        if let Some(job) = self.jobs_lock().iter_mut().find(|job| job.pid == pid) {
            job.status = JobStatus::Signaled(signal);
            return true;
        }
        false
    }

    pub fn remove_job(&self, pid: u32) -> bool {
        let mut jobs = self.jobs_lock();
        let len_before = jobs.len();
        jobs.retain(|job| job.pid != pid);
        len_before != jobs.len()
    }

    pub fn jobs(&self) -> Vec<JobEntry> {
        self.jobs_lock().clone()
    }

    // ── Hash table (PATH cache) ──

    pub fn hash_table(&self) -> &HashMap<String, String> {
        &self.hash_table
    }

    pub fn hash_insert(&mut self, name: String, path: String) {
        self.hash_table.insert(name, path);
    }

    pub fn hash_remove(&mut self, name: &str) -> bool {
        self.hash_table.remove(name).is_some()
    }

    pub fn hash_clear(&mut self) {
        self.hash_table.clear();
    }

    // ── Disabled builtins (for `enable` builtin) ──

    /// Check if a builtin is disabled.
    pub fn is_builtin_disabled(&self, name: &str) -> bool {
        self.disabled_builtins.contains(name)
    }

    /// Disable a builtin.
    pub fn disable_builtin(&mut self, name: &str) {
        self.disabled_builtins.insert(name.to_string());
    }

    /// Enable a previously disabled builtin.
    pub fn enable_builtin(&mut self, name: &str) -> bool {
        self.disabled_builtins.remove(name)
    }

    /// All disabled builtin names.
    pub fn disabled_builtins(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.disabled_builtins.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }

    // ── Functions (read-only access to map) ──

    pub fn functions(&self) -> &HashMap<String, FunctionDef> {
        &self.functions
    }

    // ── Persistence ──

    pub fn to_snapshot(&self) -> EnvSnapshot {
        // Flatten scope stack — only global scope variables persist
        let variables = self.scopes[0].clone();
        let functions = self
            .functions
            .iter()
            .map(|(name, def)| (name.clone(), def.source.clone()))
            .collect();
        let traps = self
            .traps
            .iter()
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
                    self.functions.insert(
                        name.clone(),
                        FunctionDef {
                            source: source.clone(),
                            body,
                        },
                    );
                }
                _ => {
                    // Log warning but don't fail — invalid function source is non-fatal
                    tracing::warn!("failed to re-parse function '{}' from snapshot", name);
                }
            }
        }

        // Restore traps
        for (signal, action) in &snapshot.traps {
            self.traps.insert(
                signal.clone(),
                TrapAction {
                    action: action.clone(),
                    inherited: false,
                },
            );
        }
    }

    // ── Isolation context ──

    /// Set the isolation context token for this shell environment.
    ///
    /// This is called by the daemon when creating the MASH instance.
    /// MASH never inspects this token; it is passed through to platform
    /// spawn traits when spawning child processes.
    pub fn set_isolation_context(&mut self, ctx: malt_platform::isolation::IsolationContext) {
        self.isolation_context = Some(ctx);
    }

    /// Get a reference to the isolation context token, if any.
    pub fn isolation_context(&self) -> Option<&malt_platform::isolation::IsolationContext> {
        self.isolation_context.as_ref()
    }

    /// Take the isolation context token (used when spawning processes).
    pub fn take_isolation_context(&mut self) -> Option<malt_platform::isolation::IsolationContext> {
        self.isolation_context.take()
    }

    pub fn register_fd(&self, fd: u32, file: File) {
        self.clear_fd_alias(fd);
        self.fd_registry.register_file_at(fd, file);
    }

    pub fn register_fd_alias(&self, fd: u32, target_fd: u32) {
        let _ = self.fd_registry.close(fd);
        self.fd_aliases_lock().insert(fd, target_fd);
    }

    pub fn fd_alias_target(&self, fd: u32) -> Option<u32> {
        self.fd_aliases_lock().get(&fd).copied()
    }

    pub fn fd_alias_env_spec(&self) -> Option<String> {
        let aliases = self.fd_aliases_lock();
        if aliases.is_empty() {
            return None;
        }
        let mut entries: Vec<(u32, u32)> =
            aliases.iter().map(|(fd, target)| (*fd, *target)).collect();
        entries.sort_unstable_by_key(|(fd, _)| *fd);
        Some(
            entries
                .into_iter()
                .map(|(fd, target)| format!("{fd}:{target}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    pub fn open_fd(&self, fd: u32) -> std::io::Result<File> {
        self.fd_registry.open(fd)
    }

    pub fn open_fd_read(&self, fd: u32) -> std::io::Result<File> {
        self.fd_registry.open_read(fd)
    }

    pub fn open_fd_write(&self, fd: u32) -> std::io::Result<File> {
        self.fd_registry.open_write(fd)
    }

    pub fn close_fd(&self, fd: u32) -> std::io::Result<()> {
        self.clear_fd_alias(fd);
        self.fd_registry.close(fd)
    }

    pub fn has_fd(&self, fd: u32) -> bool {
        self.fd_registry.is_registered(fd) || self.fd_aliases_lock().contains_key(&fd)
    }

    fn history_lock(&self) -> MutexGuard<'_, Vec<String>> {
        match self.history.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn jobs_lock(&self) -> MutexGuard<'_, Vec<JobEntry>> {
        match self.jobs.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn fd_aliases_lock(&self) -> MutexGuard<'_, HashMap<u32, u32>> {
        match self.fd_aliases.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn clear_fd_alias(&self, fd: u32) {
        self.fd_aliases_lock().remove(&fd);
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
