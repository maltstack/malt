//! Shell environment — variable scope stack, special parameters, options, persistence.

use crate::ast::{Command, Spanned};
use crate::parser;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{mpsc::Sender, Arc, Condvar, Mutex, MutexGuard};

const MASH_FD_ALIASES_ENV: &str = "MASH_FD_ALIASES";
const MASH_FD_SNAPSHOTS_ENV: &str = "MASH_FD_SNAPSHOTS";
const DEFAULT_IFS: &str = " \t\n";

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
    pub allexport: bool,
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
            allexport: false,
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
        if self.allexport {
            s.push('a');
        }
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
    Stopped,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BackgroundCompletion {
    done: bool,
    exit_code: i32,
}

#[derive(Debug)]
struct BackgroundTask {
    state: Mutex<BackgroundCompletion>,
    ready: Condvar,
    pending_signal: Mutex<Option<(String, i32)>>,
}

impl BackgroundTask {
    fn new() -> Self {
        Self {
            state: Mutex::new(BackgroundCompletion {
                done: false,
                exit_code: 0,
            }),
            ready: Condvar::new(),
            pending_signal: Mutex::new(None),
        }
    }

    fn request_signal(&self, signal: String, exit_code: i32) {
        let mut pending = match self.pending_signal.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *pending = Some((signal, exit_code));
    }

    fn take_signal(&self) -> Option<(String, i32)> {
        let mut pending = match self.pending_signal.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        pending.take()
    }

    fn complete(&self, exit_code: i32) {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.done {
            return;
        }
        state.done = true;
        state.exit_code = exit_code;
        self.ready.notify_all();
    }

    fn wait(&self) -> i32 {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        while !state.done {
            state = match self.ready.wait(state) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        state.exit_code
    }
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
    job_tasks: Arc<Mutex<HashMap<u32, Arc<BackgroundTask>>>>,
    /// Opaque isolation context token passed through from daemon.
    /// MASH does not interpret this; it's passed to platform spawn traits.
    isolation_context: Option<malt_platform::isolation::IsolationContext>,
    /// Windows Job Object every externally-spawned child in this session gets
    /// assigned to, if the session's isolation tier is above Bare. Shared
    /// (`Arc`) across `Env::clone()` (subshells) so the whole session's
    /// process tree lives in one job — killing the job kills all of it.
    #[cfg(windows)]
    job_object: Option<Arc<malt_platform::isolation::job_objects::JobObject>>,
    fd_registry: malt_platform::vfs::SharedFdRegistry,
    fd_aliases: Arc<Mutex<HashMap<u32, u32>>>,
    fd_snapshots: Arc<Mutex<HashMap<u32, PathBuf>>>,
    bg_pid_reporter: Option<Sender<u32>>,
    bg_pid_reporting_enabled: bool,
    current_job_id: Option<u32>,
    last_command_substitution_status: Option<i32>,
}

impl Clone for Env {
    fn clone(&self) -> Self {
        let fd_registry = malt_platform::vfs::SharedFdRegistry::new();
        for fd in self.fd_registry.list_fds() {
            if let Ok(file) = self.fd_registry.open(fd) {
                fd_registry.register_file_at(fd, file);
            }
        }

        Self {
            scopes: self.scopes.clone(),
            unset_masks: self.unset_masks.clone(),
            local_vars: self.local_vars.clone(),
            special: self.special.clone(),
            options: self.options.clone(),
            functions: self.functions.clone(),
            aliases: self.aliases.clone(),
            traps: self.traps.clone(),
            loop_control: self.loop_control.clone(),
            call_stack: self.call_stack.clone(),
            call_depth: self.call_depth,
            loop_depth: self.loop_depth,
            exit_requested: self.exit_requested,
            is_interactive: self.is_interactive,
            dir_stack: self.dir_stack.clone(),
            hash_table: self.hash_table.clone(),
            disabled_builtins: self.disabled_builtins.clone(),
            suppress_errexit: self.suppress_errexit,
            history: self.history.clone(),
            jobs: self.jobs.clone(),
            job_tasks: self.job_tasks.clone(),
            isolation_context: self.isolation_context.clone(),
            #[cfg(windows)]
            job_object: self.job_object.clone(),
            fd_registry,
            fd_aliases: Arc::new(Mutex::new(self.fd_aliases_lock().clone())),
            fd_snapshots: Arc::new(Mutex::new(self.fd_snapshots_lock().clone())),
            bg_pid_reporter: self.bg_pid_reporter.clone(),
            bg_pid_reporting_enabled: self.bg_pid_reporting_enabled,
            current_job_id: self.current_job_id,
            last_command_substitution_status: self.last_command_substitution_status,
        }
    }
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
            job_tasks: Arc::new(Mutex::new(HashMap::new())),
            isolation_context: None,
            #[cfg(windows)]
            job_object: None,
            fd_registry: malt_platform::vfs::SharedFdRegistry::new(),
            fd_aliases: Arc::new(Mutex::new(HashMap::new())),
            fd_snapshots: Arc::new(Mutex::new(HashMap::new())),
            bg_pid_reporter: None,
            bg_pid_reporting_enabled: false,
            current_job_id: None,
            last_command_substitution_status: None,
        };
        env.special
            .insert("$".to_string(), std::process::id().to_string());
        env.special.insert("?".to_string(), "0".to_string());
        let ppid = std::env::var("MASH_PPID")
            .ok()
            .or_else(|| malt_platform::process::parent_pid().map(|pid| pid.to_string()))
            .unwrap_or_else(|| "0".to_string());
        env.special.insert("PPID".to_string(), ppid);
        env.scopes[0].insert(
            "IFS".to_string(),
            Variable {
                value: VarValue::String(DEFAULT_IFS.to_string()),
                exported: false,
                readonly: false,
                integer: false,
            },
        );
        env
    }

    pub fn from_os() -> Self {
        mark_inherited_fds_cloexec();
        let mut env = Self::empty();
        for (key, value) in std::env::vars() {
            if key == "IFS" {
                continue;
            }
            #[cfg(windows)]
            let key = if key.eq_ignore_ascii_case("PATH")
                || key.eq_ignore_ascii_case("HOME")
                || key.eq_ignore_ascii_case("TEMP")
                || key.eq_ignore_ascii_case("TMP")
                || key.eq_ignore_ascii_case("COMSPEC")
                || key.eq_ignore_ascii_case("SYSTEMROOT")
                || key.eq_ignore_ascii_case("WINDIR")
                || key.eq_ignore_ascii_case("USERPROFILE")
                || key.eq_ignore_ascii_case("HOMEDRIVE")
                || key.eq_ignore_ascii_case("HOMEPATH")
                || key.eq_ignore_ascii_case("PSMODULEPATH")
                || key.eq_ignore_ascii_case("PATHEXT")
            {
                key.to_ascii_uppercase()
            } else {
                key
            };
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
        if let Ok(snapshot_spec) = std::env::var(MASH_FD_SNAPSHOTS_ENV) {
            for entry in snapshot_spec.split(',').filter(|entry| !entry.is_empty()) {
                if let Some((fd_text, path_hex)) = entry.split_once('|') {
                    if let (Ok(fd), Some(path)) =
                        (fd_text.parse::<u32>(), decode_snapshot_path(path_hex))
                    {
                        env.register_fd_snapshot_path(fd, path);
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

    fn visible_scope_index_up_to(&self, name: &str, scope_count: usize) -> Option<usize> {
        for i in (0..scope_count).rev() {
            if self.unset_masks[i].contains(name) {
                return None;
            }
            if self.scopes[i].contains_key(name) {
                return Some(i);
            }
        }
        None
    }

    pub fn set(&mut self, name: &str, mut var: Variable) -> Result<(), EnvError> {
        if let Some(existing) = self.get(name) {
            if existing.readonly {
                return Err(EnvError::ReadonlyVariable(name.to_string()));
            }
        }
        if self.options.allexport {
            var.exported = true;
        }
        let top = self.scopes.last_mut().ok_or(EnvError::EmptyScopes)?;
        top.insert(name.to_string(), var);
        self.unset_masks
            .last_mut()
            .ok_or(EnvError::EmptyScopes)?
            .remove(name);
        Ok(())
    }

    pub fn set_global(&mut self, name: &str, mut var: Variable) -> Result<(), EnvError> {
        if let Some(existing) = self.scopes[0].get(name) {
            if existing.readonly {
                return Err(EnvError::ReadonlyVariable(name.to_string()));
            }
        }
        if self.options.allexport {
            var.exported = true;
        }
        self.scopes[0].insert(name.to_string(), var);
        self.unset_masks[0].remove(name);
        Ok(())
    }

    pub fn set_local(&mut self, name: &str, mut var: Variable) -> Result<(), EnvError> {
        if let Some(existing) = self.get(name) {
            if existing.readonly {
                return Err(EnvError::ReadonlyVariable(name.to_string()));
            }
        }
        if self.options.allexport {
            var.exported = true;
        }
        self.mark_local(name);
        let top = self.scopes.last_mut().ok_or(EnvError::EmptyScopes)?;
        top.insert(name.to_string(), var);
        self.unset_masks
            .last_mut()
            .ok_or(EnvError::EmptyScopes)?
            .remove(name);
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
        let mut masked = HashSet::new();
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            for name in &self.unset_masks[i] {
                masked.insert(name.clone());
            }
            for (name, var) in scope {
                if !masked.contains(name) && var.readonly {
                    result.insert(name.clone(), var.as_str().to_string());
                    masked.insert(name.clone());
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

    pub fn pop_scope_with_merge(&mut self) -> Result<(), EnvError> {
        if self.scopes.len() <= 1 {
            return Err(EnvError::EmptyScopes);
        }

        let scope = self.scopes.pop().ok_or(EnvError::EmptyScopes)?;
        let unset_mask = self.unset_masks.pop().ok_or(EnvError::EmptyScopes)?;
        let local_vars = self.local_vars.pop().ok_or(EnvError::EmptyScopes)?;
        let parent_idx = self.scopes.len() - 1;
        let staged_names: HashSet<String> = scope.keys().cloned().collect();

        for (name, var) in scope {
            if local_vars.contains(&name) {
                continue;
            }
            let target_idx = if self.scopes[parent_idx].contains_key(&name)
                || self.local_vars[parent_idx].contains(&name)
                || self.unset_masks[parent_idx].contains(&name)
            {
                parent_idx
            } else {
                self.visible_scope_index_up_to(&name, self.scopes.len())
                    .unwrap_or(0)
            };
            self.scopes[target_idx].insert(name.clone(), var);
            self.unset_masks[target_idx].remove(&name);
        }

        for name in unset_mask {
            if local_vars.contains(&name) || staged_names.contains(&name) {
                continue;
            }
            let target_idx = if self.scopes[parent_idx].contains_key(&name)
                || self.local_vars[parent_idx].contains(&name)
                || self.unset_masks[parent_idx].contains(&name)
            {
                Some(parent_idx)
            } else {
                self.visible_scope_index_up_to(&name, self.scopes.len())
            };
            if let Some(idx) = target_idx {
                self.scopes[idx].remove(&name);
                self.unset_masks[idx].insert(name);
            }
        }

        Ok(())
    }

    // ── Bulk access ──

    pub fn exported_vars(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();
        let mut masked = HashSet::new();
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            for name in &self.unset_masks[i] {
                masked.insert(name.clone());
            }
            for (name, var) in scope {
                if !masked.contains(name) && var.exported {
                    result.insert(name.clone(), var.as_str().to_string());
                    masked.insert(name.clone());
                }
            }
        }
        result
    }

    pub fn all_variables(&self) -> HashMap<String, &Variable> {
        let mut result = HashMap::new();
        let mut masked = HashSet::new();
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            for name in &self.unset_masks[i] {
                masked.insert(name.clone());
            }
            for (name, var) in scope {
                if !masked.contains(name) {
                    result.insert(name.clone(), var);
                    masked.insert(name.clone());
                }
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

    pub fn set_shell_pid(&mut self, pid: u32) {
        self.special.insert("$".to_string(), pid.to_string());
    }

    pub fn set_bg_pid_reporter(&mut self, reporter: Option<Sender<u32>>) {
        self.bg_pid_reporter = reporter;
    }

    pub fn set_bg_pid_reporting_enabled(&mut self, enabled: bool) {
        self.bg_pid_reporting_enabled = enabled;
    }

    pub fn report_bg_pid(&self, pid: u32) {
        if !self.bg_pid_reporting_enabled {
            return;
        }
        if let Some(reporter) = &self.bg_pid_reporter {
            let _ = reporter.send(pid);
        }
    }

    pub fn set_current_job_id(&mut self, job_id: Option<u32>) {
        self.current_job_id = job_id;
    }

    pub fn current_job_id(&self) -> Option<u32> {
        self.current_job_id
    }

    pub fn set_last_command_substitution_status(&mut self, status: Option<i32>) {
        self.last_command_substitution_status = status;
    }

    pub fn take_last_command_substitution_status(&mut self) -> Option<i32> {
        self.last_command_substitution_status.take()
    }

    pub fn take_pending_job_signal(&self) -> Option<(String, i32)> {
        let job_id = self.current_job_id?;
        let task = self.job_tasks_lock().get(&job_id).cloned()?;
        task.take_signal()
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

    pub fn inherit_traps_for_subshell(&mut self) {
        for trap in self.traps.values_mut() {
            trap.inherited = true;
        }
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
        self.job_tasks_lock()
            .insert(job_id, Arc::new(BackgroundTask::new()));
    }

    pub fn next_job_id(&self) -> u32 {
        let jobs = self.jobs_lock();
        let mut candidate = 1u32;
        loop {
            if jobs.iter().all(|job| job.job_id != candidate) {
                return candidate;
            }
            candidate = candidate.saturating_add(1);
        }
    }

    pub fn update_job_pid(&self, job_id: u32, pid: u32) {
        if let Some(job) = self.jobs_lock().iter_mut().find(|job| job.job_id == job_id) {
            job.pid = pid;
        }
    }

    pub fn mark_job_done(&self, job_id: u32, exit_code: i32) {
        if let Some(job) = self.jobs_lock().iter_mut().find(|job| job.job_id == job_id) {
            if matches!(job.status, JobStatus::Running | JobStatus::Stopped) {
                job.status = JobStatus::Done;
            }
        }
        self.complete_job(job_id, exit_code);
    }

    pub fn signal_job(&self, pid: u32, signal: String, exit_code: i32) -> bool {
        if let Some(job) = self.jobs_lock().iter_mut().find(|job| job.pid == pid) {
            let job_id = job.job_id;
            match signal.as_str() {
                "TSTP" => {
                    job.status = JobStatus::Stopped;
                }
                "CONT" => {
                    job.status = JobStatus::Running;
                }
                _ => {
                    job.status = JobStatus::Signaled(signal.clone());
                }
            }
            if let Some(task) = self.job_tasks_lock().get(&job_id).cloned() {
                task.request_signal(signal, exit_code);
            }
            return true;
        }
        false
    }

    pub fn remove_job(&self, pid: u32) -> bool {
        let mut jobs = self.jobs_lock();
        let len_before = jobs.len();
        let removed_job_ids: Vec<u32> = jobs
            .iter()
            .filter(|job| job.pid == pid)
            .map(|job| job.job_id)
            .collect();
        jobs.retain(|job| job.pid != pid);
        if !removed_job_ids.is_empty() {
            let mut tasks = self.job_tasks_lock();
            for job_id in removed_job_ids {
                tasks.remove(&job_id);
            }
        }
        len_before != jobs.len()
    }

    pub fn jobs(&self) -> Vec<JobEntry> {
        self.jobs_lock().clone()
    }

    pub fn wait_for_job(&self, pid: u32) -> Option<i32> {
        let job_id = self
            .jobs_lock()
            .iter()
            .find(|job| job.pid == pid)
            .map(|job| job.job_id)?;
        let task = self.job_tasks_lock().get(&job_id).cloned()?;
        Some(task.wait())
    }

    pub fn job_pid_from_spec(&self, spec: &str) -> Option<u32> {
        if let Some(rest) = spec.strip_prefix('%') {
            let job_id = rest.parse::<u32>().ok()?;
            return self
                .jobs_lock()
                .iter()
                .find(|job| job.job_id == job_id)
                .map(|job| job.pid);
        }
        spec.parse::<u32>().ok()
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

        // Re-parse function source texts. `source` is the *whole* definition
        // statement ("name() { body }"), matching how it was captured in
        // executor.rs's `Command::FunctionDef` handling — so the reparsed
        // top-level command is itself a `FunctionDef` node, and the inner
        // `body` must be extracted from it. Storing the whole reparsed node
        // as `FunctionDef.body` (as this used to do) would make every
        // restored function's body a `FunctionDef` wrapping the real body
        // instead of the real body itself.
        for (name, source) in &snapshot.functions {
            match parser::parse(source) {
                Ok(mut cmds) if !cmds.is_empty() => {
                    let parsed = cmds.remove(0);
                    match &parsed.node {
                        Command::FunctionDef { body, .. } => {
                            self.functions.insert(
                                name.clone(),
                                FunctionDef {
                                    source: source.clone(),
                                    body: body.as_ref().clone(),
                                },
                            );
                        }
                        _ => {
                            tracing::warn!(
                                "function '{}' snapshot source did not reparse as a function definition",
                                name
                            );
                        }
                    }
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

    /// Set the Windows Job Object every externally-spawned child in this
    /// session should be assigned to. Called once by the daemon at session
    /// startup when the isolation tier is above Bare; shared across
    /// subshells via `Env::clone()`.
    #[cfg(windows)]
    pub fn set_job_object(&mut self, job: Arc<malt_platform::isolation::job_objects::JobObject>) {
        self.job_object = Some(job);
    }

    /// Get the session's Job Object, if isolation is active.
    #[cfg(windows)]
    pub fn job_object(&self) -> Option<&Arc<malt_platform::isolation::job_objects::JobObject>> {
        self.job_object.as_ref()
    }

    pub fn register_fd(&self, fd: u32, file: File) {
        self.clear_fd_alias(fd);
        self.clear_fd_snapshot(fd);
        self.fd_registry.register_file_at(fd, file);
    }

    pub fn register_fd_alias(&self, fd: u32, target_fd: u32) {
        let _ = self.fd_registry.close(fd);
        self.clear_fd_snapshot(fd);
        self.fd_aliases_lock().insert(fd, target_fd);
    }

    pub fn register_fd_snapshot_path(&self, fd: u32, path: PathBuf) {
        let _ = self.fd_registry.close(fd);
        self.clear_fd_alias(fd);
        self.fd_snapshots_lock().insert(fd, path);
    }

    pub fn fd_alias_target(&self, fd: u32) -> Option<u32> {
        self.fd_aliases_lock().get(&fd).copied()
    }

    pub fn fd_alias_env_spec(&self) -> Option<String> {
        self.fd_alias_env_spec_filtered(|_| true)
    }

    pub fn nonstdio_fd_alias_env_spec(&self) -> Option<String> {
        self.fd_alias_env_spec_filtered(|fd| fd > 2)
    }

    fn fd_alias_env_spec_filtered(&self, include: impl Fn(u32) -> bool) -> Option<String> {
        let aliases = self.fd_aliases_lock();
        let mut entries: Vec<(u32, u32)> = aliases
            .iter()
            .filter_map(|(fd, target)| include(*fd).then_some((*fd, *target)))
            .collect();
        if entries.is_empty() {
            return None;
        }
        entries.sort_unstable_by_key(|(fd, _)| *fd);
        Some(
            entries
                .into_iter()
                .map(|(fd, target)| format!("{fd}:{target}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    pub fn fd_snapshot_path(&self, fd: u32) -> Option<PathBuf> {
        self.fd_snapshots_lock().get(&fd).cloned()
    }

    pub fn fd_snapshot_env_spec(&self) -> Option<String> {
        self.fd_snapshot_env_spec_filtered(|_| true)
    }

    pub fn nonstdio_fd_snapshot_env_spec(&self) -> Option<String> {
        self.fd_snapshot_env_spec_filtered(|fd| fd > 2)
    }

    fn fd_snapshot_env_spec_filtered(&self, include: impl Fn(u32) -> bool) -> Option<String> {
        let snapshots = self.fd_snapshots_lock();
        let mut entries: Vec<(u32, PathBuf)> = snapshots
            .iter()
            .filter_map(|(fd, path)| include(*fd).then_some((*fd, path.clone())))
            .collect();
        if entries.is_empty() {
            return None;
        }
        entries.sort_unstable_by_key(|(fd, _)| *fd);
        Some(
            entries
                .into_iter()
                .map(|(fd, path)| format!("{fd}|{}", encode_snapshot_path(&path)))
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    pub fn open_fd(&self, fd: u32) -> std::io::Result<File> {
        let fd = self.resolve_fd_target(fd).unwrap_or(fd);
        if let Some(path) = self.fd_snapshot_path(fd) {
            return std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path);
        }
        self.fd_registry.open(fd)
    }

    pub fn open_fd_read(&self, fd: u32) -> std::io::Result<File> {
        let fd = self.resolve_fd_target(fd).unwrap_or(fd);
        if let Some(path) = self.fd_snapshot_path(fd) {
            return std::fs::OpenOptions::new().read(true).open(path);
        }
        self.fd_registry.open_read(fd)
    }

    pub fn open_fd_write(&self, fd: u32) -> std::io::Result<File> {
        let fd = self.resolve_fd_target(fd).unwrap_or(fd);
        if let Some(path) = self.fd_snapshot_path(fd) {
            let mut file = std::fs::OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::End(0))?;
            return Ok(file);
        }
        self.fd_registry.open_write(fd)
    }

    pub fn close_fd(&self, fd: u32) -> std::io::Result<()> {
        self.clear_fd_alias(fd);
        self.clear_fd_snapshot(fd);
        self.fd_registry.close(fd)
    }

    pub fn has_fd(&self, fd: u32) -> bool {
        self.fd_registry.is_registered(fd)
            || self.fd_aliases_lock().contains_key(&fd)
            || self.fd_snapshots_lock().contains_key(&fd)
    }

    pub fn nonstdio_fds(&self) -> Vec<u32> {
        let mut fds: Vec<u32> = self
            .fd_registry
            .list_fds()
            .into_iter()
            .filter(|fd| *fd > 2)
            .collect();
        fds.extend(self.fd_aliases_lock().keys().copied().filter(|fd| *fd > 2));
        fds.extend(
            self.fd_snapshots_lock()
                .keys()
                .copied()
                .filter(|fd| *fd > 2),
        );
        fds.sort_unstable();
        fds.dedup();
        fds
    }

    fn resolve_fd_target(&self, fd: u32) -> Option<u32> {
        let mut current = fd;
        for _ in 0..64 {
            let Some(next) = self.fd_alias_target(current) else {
                return Some(current);
            };
            if next == current {
                return Some(next);
            }
            current = next;
        }
        None
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

    fn job_tasks_lock(&self) -> MutexGuard<'_, HashMap<u32, Arc<BackgroundTask>>> {
        match self.job_tasks.lock() {
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

    fn fd_snapshots_lock(&self) -> MutexGuard<'_, HashMap<u32, PathBuf>> {
        match self.fd_snapshots.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn complete_job(&self, job_id: u32, exit_code: i32) {
        if let Some(task) = self.job_tasks_lock().get(&job_id).cloned() {
            task.complete(exit_code);
        }
    }

    fn clear_fd_alias(&self, fd: u32) {
        self.fd_aliases_lock().remove(&fd);
    }

    fn clear_fd_snapshot(&self, fd: u32) {
        self.fd_snapshots_lock().remove(&fd);
    }
}

#[cfg(unix)]
fn mark_inherited_fds_cloexec() {
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;

    let mark_fd = |fd: i32| {
        if fd <= 2 {
            return;
        }
        // SAFETY: `fcntl(fd, F_GETFD)` and `F_SETFD` are process-local metadata ops.
        let flags = unsafe { fcntl(fd, F_GETFD) };
        if flags >= 0 {
            // SAFETY: same as above; preserving existing flags and adding CLOEXEC.
            let _ = unsafe { fcntl(fd, F_SETFD, flags | FD_CLOEXEC) };
        }
    };

    // Prefer scanning real open descriptors so high-number inherited fds are covered.
    if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Ok(fd) = name.parse::<i32>() {
                    mark_fd(fd);
                }
            }
        }
        return;
    }

    // Fallback when /proc is unavailable.
    for fd in 3..=1024 {
        mark_fd(fd);
    }
}

#[cfg(not(unix))]
fn mark_inherited_fds_cloexec() {}

fn encode_snapshot_path(path: &Path) -> String {
    path.as_os_str()
        .to_string_lossy()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_snapshot_path(hex: &str) -> Option<PathBuf> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for idx in (0..hex.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&hex[idx..idx + 2], 16).ok()?);
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
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
