//! Executor — walks the AST and runs commands.
//!
//! This is the scaffold: simple external commands, AND-OR lists, brace groups,
//! env assignments. Pipelines, redirects, control flow, and builtins are
//! added in subsequent tasks.

use std::io::Read as IoRead;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::{collections::HashMap, fs::File};
use std::{
    io::Seek,
    io::SeekFrom,
    sync::atomic::{AtomicU64, Ordering},
    sync::mpsc,
    time::{Duration, Instant},
};

use crate::ast::{Command, ListOp, Redirect, RedirectKind, Span, Spanned};
use crate::env::{CallFrame, Env, EnvError, LoopControl, TrapAction, Variable};
use crate::expander;

// ── ExecResult ─────────────────────────────────────────────────────────

/// Result of executing a command.
#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    /// Exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout bytes (only populated when stdout is piped).
    pub stdout: Vec<u8>,
    /// Captured stderr bytes (only populated when stderr is piped).
    pub stderr: Vec<u8>,
}

impl ExecResult {
    fn success() -> Self {
        Self {
            exit_code: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn with_code(code: i32) -> Self {
        Self {
            exit_code: code,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn failure(code: i32, msg: impl Into<String>) -> Self {
        Self {
            exit_code: code,
            stdout: Vec::new(),
            stderr: msg.into().into_bytes(),
        }
    }
}

fn noninteractive_shell_error(env: &mut Env, msg: impl Into<String>) -> ExecResult {
    let result = ExecResult::failure(1, msg);
    if !env.is_interactive() {
        env.request_exit(result.exit_code);
    }
    result
}

fn redirect_error_aborts_noninteractive_shell(
    env: &mut Env,
    cmd_name: &str,
    err_result: &ExecResult,
) -> bool {
    if env.is_interactive() {
        return false;
    }
    if is_special_builtin_name(cmd_name) {
        return true;
    }
    let stderr = String::from_utf8_lossy(&err_result.stderr);
    stderr.contains("heredoc expansion:")
        || stderr.contains("bad substitution")
        || stderr.contains("parameter not set")
        || stderr.contains("parameter null or not set")
}

// ── Public API ─────────────────────────────────────────────────────────

/// Execute a single command node.
pub fn execute(cmd: &Spanned<Command>, source: &str, env: &mut Env) -> ExecResult {
    let result = execute_inner(cmd, source, env);
    env.set_exit_code(result.exit_code);
    result
}

/// Execute a list of commands sequentially, accumulating output.
pub fn execute_list(commands: &[Spanned<Command>], source: &str, env: &mut Env) -> ExecResult {
    let mut shell_capture = install_shell_stdio_capture(env);
    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();
    let mut last_code = 0i32;

    for cmd in commands {
        // Check for exit request between commands.
        if env.exit_requested().is_some() {
            break;
        }
        // Check for loop control (break/continue/return).
        if !matches!(env.loop_control(), LoopControl::None) {
            break;
        }

        if env.is_interactive() && !env.options().nolog {
            env.push_history_entry(cmd.span.text(source).to_string());
        }

        let prev_suppress_errexit = env.suppress_errexit();
        let result = execute(cmd, source, env);
        let suppress_errexit = env.suppress_errexit();
        env.set_suppress_errexit(prev_suppress_errexit);
        last_code = result.exit_code;
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);
        all_stdout.extend_from_slice(&shell_capture.drain_stdout_bytes());
        all_stderr.extend_from_slice(&shell_capture.drain_stderr_bytes());

        // set -e (errexit): abort on non-zero exit code.
        if env.options().errexit && !suppress_errexit && last_code != 0 {
            env.request_exit(last_code);
            break;
        }
    }

    if let Some(code) = env.exit_requested() {
        last_code = code;
    }

    if let Some(trap) = env.get_trap("EXIT").cloned() {
        if trap.inherited {
            all_stdout.extend_from_slice(&shell_capture.stdout_bytes(env));
            all_stderr.extend_from_slice(&shell_capture.stderr_bytes(env));
            return ExecResult {
                exit_code: last_code,
                stdout: all_stdout,
                stderr: all_stderr,
            };
        }
        let saved_loop_control = env.loop_control().clone();
        let saved_exit_requested = env.exit_requested();
        env.set_loop_control(LoopControl::None);
        env.set_exit_requested(None);
        let trap_result = execute_trap_action(&trap.action, env);
        let trap_exit_requested = env.exit_requested();
        env.set_loop_control(saved_loop_control.clone());
        env.set_exit_requested(saved_exit_requested.or(trap_exit_requested));
        all_stdout.extend_from_slice(&trap_result.stdout);
        all_stderr.extend_from_slice(&trap_result.stderr);
        if let Some(code) = trap_exit_requested.or(saved_exit_requested) {
            last_code = code;
        } else {
            last_code = trap_result.exit_code;
        }
    }

    all_stdout.extend_from_slice(&shell_capture.stdout_bytes(env));
    all_stderr.extend_from_slice(&shell_capture.stderr_bytes(env));

    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

fn execute_list_with_io(
    commands: &[Spanned<Command>],
    source: &str,
    env: &mut Env,
    stdin_file: Option<&File>,
    stdout_file: Option<&File>,
) -> ExecResult {
    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();
    let mut last_code = 0i32;

    for cmd in commands {
        if env.exit_requested().is_some() {
            break;
        }
        if !matches!(env.loop_control(), LoopControl::None) {
            break;
        }

        let stdin = stdin_file.and_then(|file| file.try_clone().ok());
        let stdout = stdout_file.and_then(|file| file.try_clone().ok());
        let prev_suppress_errexit = env.suppress_errexit();
        let result = execute_with_io(cmd, source, env, stdin, stdout);
        let suppress_errexit = env.suppress_errexit();
        env.set_suppress_errexit(prev_suppress_errexit);
        last_code = result.exit_code;
        env.set_exit_code(last_code);
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        if env.options().errexit && !suppress_errexit && last_code != 0 {
            env.request_exit(last_code);
            break;
        }
    }

    if let Some(code) = env.exit_requested() {
        last_code = code;
    }

    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

struct ShellStdioCapture {
    stdout: Option<CaptureFile>,
    stderr: Option<CaptureFile>,
}

struct CaptureFile {
    fd: u32,
    path: PathBuf,
    read_pos: u64,
}

impl ShellStdioCapture {
    fn drain_stdout_bytes(&mut self) -> Vec<u8> {
        Self::drain_capture(self.stdout.as_mut())
    }

    fn drain_stderr_bytes(&mut self) -> Vec<u8> {
        Self::drain_capture(self.stderr.as_mut())
    }

    fn stdout_bytes(&mut self, env: &Env) -> Vec<u8> {
        let bytes = self.drain_stdout_bytes();
        let capture = self.stdout.take();
        Self::close_capture(1, env, capture);
        bytes
    }

    fn stderr_bytes(&mut self, env: &Env) -> Vec<u8> {
        let bytes = self.drain_stderr_bytes();
        let capture = self.stderr.take();
        Self::close_capture(2, env, capture);
        bytes
    }

    fn drain_capture(capture: Option<&mut CaptureFile>) -> Vec<u8> {
        let Some(capture) = capture else {
            return Vec::new();
        };
        let mut reader = match std::fs::OpenOptions::new().read(true).open(&capture.path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };
        if reader.seek(SeekFrom::Start(capture.read_pos)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        if reader.read_to_end(&mut bytes).is_err() {
            bytes.clear();
            return bytes;
        }
        capture.read_pos = capture.read_pos.saturating_add(bytes.len() as u64);
        bytes
    }

    fn close_capture(stdio_fd: u32, env: &Env, capture: Option<CaptureFile>) {
        let Some(capture) = capture else {
            return;
        };
        let _ = env.close_fd(capture.fd);
        let _ = std::fs::remove_file(&capture.path);
        if env.has_fd(stdio_fd) {
            let _ = env.close_fd(stdio_fd);
        }
    }
}

fn install_shell_stdio_capture(env: &Env) -> ShellStdioCapture {
    ShellStdioCapture {
        stdout: install_shell_capture_fd(env, 1),
        stderr: install_shell_capture_fd(env, 2),
    }
}

fn install_shell_capture_fd(env: &Env, fd: u32) -> Option<CaptureFile> {
    if env.has_fd(fd) {
        return None;
    }

    let path = shell_capture_path(fd);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .ok()?;
    drop(file);
    env.register_fd_snapshot_path(fd, path.clone());
    Some(CaptureFile {
        fd,
        path,
        read_pos: 0,
    })
}

fn shell_capture_path(fd: u32) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("mash-shell-captures");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!(
        "mash-shell-capture-{}-{}-{}.tmp",
        std::process::id(),
        id,
        fd
    ))
}

/// Execute a command string and capture stdout. Used by expander for $(cmd).
/// Runs in a cloned Env (subshell semantics). Strips trailing newlines.
pub fn capture_command(
    cmd_str: &str,
    env: &mut Env,
) -> Result<String, crate::expander::ExpandError> {
    let cmds = match crate::parser::parse(cmd_str) {
        Ok(cmds) => cmds,
        Err(e) => {
            return Err(crate::expander::ExpandError::CommandSubstitution(
                e.to_string(),
            ))
        }
    };
    let mut sub_env = env.clone();
    let _ = sub_env.close_fd(1);
    sub_env.clear_noninherited_traps();
    let result = execute_list(&cmds, cmd_str, &mut sub_env);
    env.set_exit_code(result.exit_code);
    let mut output = String::from_utf8_lossy(&result.stdout).to_string();
    // Strip trailing newlines (POSIX).
    while output.ends_with('\n') {
        output.pop();
    }
    while output.ends_with('\r') {
        output.pop();
    }
    Ok(output)
}

// ── Dispatch ───────────────────────────────────────────────────────────

fn execute_inner(cmd: &Spanned<Command>, source: &str, env: &mut Env) -> ExecResult {
    match &cmd.node {
        Command::Empty => ExecResult::success(),

        Command::Simple {
            name,
            args,
            redirects,
            env_assigns,
        } => execute_simple(name, args, redirects, env_assigns, source, env),

        Command::EnvAssign { assigns } => execute_env_assign(assigns, source, env),

        Command::List { pairs, last } => execute_list_node(pairs, last, source, env),

        Command::BraceGroup { body } => execute_list(body, source, env),

        Command::Subshell { body } => {
            // Clone env so changes in the subshell don't affect the parent.
            let mut sub_env = env.clone();
            if !env.options().nonlexicalctrl {
                sub_env.set_loop_depth(0);
            }
            sub_env.inherit_traps_for_subshell();
            let result = execute_list(body, source, &mut sub_env);
            // Propagate exit code to parent env (for return to capture).
            env.set_exit_code(result.exit_code);
            result
        }

        Command::Pipeline { commands, negated } => {
            execute_pipeline(commands, *negated, source, env)
        }

        Command::Background(inner) => {
            // Run async work in a detached shell context but preserve shared job state.
            let bg_id = env.next_job_id();
            let mut bg_env = env.clone();
            bg_env.set_shell_pid(bg_id);
            bg_env.set_current_job_id(Some(bg_id));
            // Async jobs do not inherit trap actions; keep only ignored traps.
            let inherited_actions: Vec<String> = bg_env
                .traps()
                .iter()
                .filter_map(|(signal, trap)| {
                    if trap.action.is_empty() {
                        None
                    } else {
                        Some(signal.clone())
                    }
                })
                .collect();
            for signal in inherited_actions {
                bg_env.clear_trap(&signal);
            }
            // Async command lists ignore INT/QUIT by default unless explicitly reset.
            if bg_env.get_trap("INT").is_none() {
                bg_env.set_trap(
                    "INT".to_string(),
                    TrapAction {
                        action: String::new(),
                        inherited: true,
                    },
                );
            }
            if bg_env.get_trap("QUIT").is_none() {
                bg_env.set_trap(
                    "QUIT".to_string(),
                    TrapAction {
                        action: String::new(),
                        inherited: true,
                    },
                );
            }
            let report_spawned_child_pid = matches!(
                &inner.node,
                Command::Simple { .. } | Command::Pipeline { .. }
            ) || matches!(
                &inner.node,
                Command::Redirected { cmd, .. }
                    if matches!(&cmd.node, Command::Simple { .. } | Command::Pipeline { .. })
            );
            let (bg_pid_tx, bg_pid_rx) = mpsc::channel();
            if report_spawned_child_pid {
                bg_env.set_bg_pid_reporter(Some(bg_pid_tx));
                bg_env.set_bg_pid_reporting_enabled(true);
            } else {
                bg_env.set_bg_pid_reporter(None);
                bg_env.set_bg_pid_reporting_enabled(false);
            }
            let bg_source = source.to_string();
            let bg_cmd = inner.as_ref().clone();
            env.register_job(bg_id, inner.span.text(source).trim().to_string());
            std::thread::spawn(move || {
                let result = execute(&bg_cmd, &bg_source, &mut bg_env);
                write_background_result(&bg_env, &result);
                bg_env.mark_job_done(bg_id, result.exit_code);
            });
            let bg_pid = if report_spawned_child_pid {
                bg_pid_rx
                    .recv_timeout(Duration::from_millis(500))
                    .unwrap_or(bg_id)
            } else {
                bg_id
            };
            env.update_job_pid(bg_id, bg_pid);
            env.set_last_bg_pid(bg_pid);
            ExecResult::success()
        }

        Command::Redirected {
            cmd: inner,
            redirects,
        } => {
            let mut resolved_io = match resolve_redirects(redirects, source, env) {
                Ok(io) => io,
                Err(err_result) => {
                    if matches!(&inner.node, Command::Simple { name, .. } if {
                        let cmd_text = name.text(source);
                        let stderr = String::from_utf8_lossy(&err_result.stderr);
                        !env.is_interactive()
                            && (is_special_builtin_name(cmd_text)
                                || stderr.contains("heredoc expansion:")
                                || stderr.contains("bad substitution")
                                || stderr.contains("parameter not set")
                                || stderr.contains("parameter null or not set"))
                    }) {
                        env.request_exit(err_result.exit_code);
                    }
                    return err_result;
                }
            };
            let saved_states: Vec<(u32, SavedFdState)> = nonstdio_affected_fds(&resolved_io)
                .into_iter()
                .map(|fd| (fd, save_fd_state(env, fd)))
                .collect();
            apply_nonstdio_redirects(env, &mut resolved_io);
            // Execute the inner command, capturing its output.
            let mut result = execute(inner, source, env);
            // Apply redirects: write captured output to redirect files.
            apply_output_redirects(&mut result, resolved_io);
            for (fd, state) in saved_states {
                restore_fd_state(env, fd, state);
            }
            result
        }

        Command::If {
            condition,
            then_body,
            elif_clauses,
            else_body,
        } => execute_if(
            condition,
            then_body,
            elif_clauses,
            else_body.as_deref(),
            source,
            env,
        ),

        Command::While { condition, body } => {
            execute_while_until(condition, body, /* is_until */ false, source, env)
        }

        Command::Until { condition, body } => {
            execute_while_until(condition, body, /* is_until */ true, source, env)
        }

        Command::For { var, words, body } => execute_for(var, words, body, source, env),

        Command::ForArith {
            init,
            cond,
            step,
            body,
        } => execute_for_arith(init, cond, step, body, source, env),

        Command::Case { word, items } => execute_case(word, items, source, env),

        Command::Select { var, words, body } => {
            // Select requires interactive input. Stub with code 1.
            let _ = (var, words, body);
            ExecResult::failure(
                1,
                "mash: select: not yet implemented (requires interactive input)\n",
            )
        }

        Command::FunctionDef { name, body } => {
            let func_name = name.text(source).to_string();
            // Store the full original source so body spans remain valid.
            env.define_function(func_name, source.to_string(), body.as_ref().clone());
            hash_commands_in_function_body(body, source, env);
            ExecResult::success()
        }

        Command::Arithmetic { expr } => execute_arithmetic(expr, source, env),

        Command::Conditional { expr } => execute_conditional(expr, source, env),

        Command::Coproc { name, cmd: inner } => {
            let _ = name;
            // Coproc requires bidirectional pipe management. Stub.
            let result = execute(inner, source, env);
            result
        }

        Command::Time {
            posix_format,
            command,
        } => {
            let _ = posix_format;
            // Time: just execute the command (timing output comes later).
            execute(command, source, env)
        }

        // Safety: all Command variants are covered above.
        #[allow(unreachable_patterns)]
        _ => {
            let msg = format!("mash: not yet implemented: {:?}\n", variant_name(&cmd.node));
            ExecResult::failure(127, msg)
        }
    }
}

fn record_hashed_command(env: &mut Env, name: &str, resolved: &str) {
    if env.options().hash_cmds {
        env.hash_insert(name.to_string(), resolved.to_string());
    }
}

/// Return a human-readable name for a Command variant.
fn variant_name(cmd: &Command) -> &'static str {
    match cmd {
        Command::Empty => "Empty",
        Command::Simple { .. } => "Simple",
        Command::Pipeline { .. } => "Pipeline",
        Command::List { .. } => "List",
        Command::If { .. } => "If",
        Command::While { .. } => "While",
        Command::Until { .. } => "Until",
        Command::For { .. } => "For",
        Command::ForArith { .. } => "ForArith",
        Command::Case { .. } => "Case",
        Command::Select { .. } => "Select",
        Command::FunctionDef { .. } => "FunctionDef",
        Command::BraceGroup { .. } => "BraceGroup",
        Command::Subshell { .. } => "Subshell",
        Command::Arithmetic { .. } => "Arithmetic",
        Command::Conditional { .. } => "Conditional",
        Command::Background(_) => "Background",
        Command::EnvAssign { .. } => "EnvAssign",
        Command::Coproc { .. } => "Coproc",
        Command::Time { .. } => "Time",
        Command::Redirected { .. } => "Redirected",
    }
}

fn hash_commands_in_function_body(body: &Spanned<Command>, source: &str, env: &mut Env) {
    if !env.options().hash_cmds {
        return;
    }
    collect_hashed_commands(body, source, env);
}

fn collect_hashed_commands(cmd: &Spanned<Command>, source: &str, env: &mut Env) {
    match &cmd.node {
        Command::Simple { name, .. } => {
            let name_text = name.text(source);
            if let Ok(expanded) = expander::expand_word(name_text, env) {
                if let Some(cmd_name) = expanded.first().filter(|name| !name.is_empty()) {
                    let dispatch_name = explicit_internal_command_name(cmd_name)
                        .unwrap_or_else(|| cmd_name.clone());
                    let tools_registry = malt_tools::Registry::new();
                    if BUILTIN_NAMES.contains(&dispatch_name.as_str())
                        || tools_registry.contains(&dispatch_name)
                    {
                        record_hashed_command(env, cmd_name, &dispatch_name);
                    } else if let Some(path) = find_in_path(cmd_name, env) {
                        record_hashed_command(env, cmd_name, &path.to_string_lossy());
                    }
                }
            }
        }
        Command::List { pairs, last } => {
            for (left, _) in pairs {
                collect_hashed_commands(left, source, env);
            }
            collect_hashed_commands(last, source, env);
        }
        Command::BraceGroup { body }
        | Command::Subshell { body }
        | Command::Pipeline { commands: body, .. } => {
            for command in body {
                collect_hashed_commands(command, source, env);
            }
        }
        Command::Background(inner)
        | Command::Redirected { cmd: inner, .. }
        | Command::Time { command: inner, .. } => collect_hashed_commands(inner, source, env),
        Command::If {
            condition,
            then_body,
            elif_clauses,
            else_body,
        } => {
            collect_hashed_commands(condition, source, env);
            for command in then_body {
                collect_hashed_commands(command, source, env);
            }
            for (elif_condition, elif_body) in elif_clauses {
                collect_hashed_commands(elif_condition, source, env);
                for command in elif_body {
                    collect_hashed_commands(command, source, env);
                }
            }
            if let Some(else_body) = else_body {
                for command in else_body {
                    collect_hashed_commands(command, source, env);
                }
            }
        }
        Command::While { condition, body } | Command::Until { condition, body } => {
            collect_hashed_commands(condition, source, env);
            for command in body {
                collect_hashed_commands(command, source, env);
            }
        }
        Command::For { body, .. }
        | Command::ForArith { body, .. }
        | Command::Select { body, .. } => {
            for command in body {
                collect_hashed_commands(command, source, env);
            }
        }
        Command::Case { items, .. } => {
            for item in items {
                for command in &item.body {
                    collect_hashed_commands(command, source, env);
                }
            }
        }
        Command::FunctionDef { body, .. } => collect_hashed_commands(body, source, env),
        Command::Coproc { cmd, .. } => collect_hashed_commands(cmd, source, env),
        Command::Empty
        | Command::EnvAssign { .. }
        | Command::Arithmetic { .. }
        | Command::Conditional { .. } => {}
    }
}

// ── Pipeline ──────────────────────────────────────────────────────────

/// Execute a pipeline: connect N commands with OS pipes via std::thread.
///
/// Each stage runs in its own thread with a cloned `Env` (pipeline stages
/// are subshells). Inter-stage data flows through OS pipes — for external
/// commands the pipe fds are passed directly to `SpawnConfig`, avoiding
/// user-space buffering.
fn execute_pipeline(
    commands: &[Spanned<Command>],
    negated: bool,
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let n = commands.len();
    if n == 0 {
        return ExecResult::success();
    }
    // Degenerate pipeline (single command) — no pipes needed.
    if n == 1 {
        let mut result = execute(&commands[0], source, env);
        if negated {
            result.exit_code = if result.exit_code == 0 { 1 } else { 0 };
        }
        return result;
    }

    // 1. Create N-1 pipe pairs.
    let mut pipes: Vec<(std::fs::File, std::fs::File)> = Vec::with_capacity(n - 1);
    for _ in 0..n - 1 {
        match malt_platform::io::create_pipe() {
            Ok(pair) => pipes.push(pair),
            Err(e) => {
                return ExecResult::failure(1, format!("mash: pipe: {e}\n"));
            }
        }
    }

    // 2. Build per-stage data and spawn threads.
    //
    // We need to move pipe ends into the threads, so we extract them from
    // the Vec now. Parent-side ends that must be dropped are collected
    // separately.
    //
    // Stage i reads from pipe[i-1].read (except stage 0: inherited/None)
    // Stage i writes to pipe[i].write (except last stage: capture to Vec<u8>)

    // Separate the pipe ends: each pipe has (read_end, write_end).
    // - read_end[i]  goes to stage i+1 as stdin
    // - write_end[i] goes to stage i   as stdout
    let mut read_ends: Vec<Option<std::fs::File>> = pipes.iter_mut().map(|_| None).collect();
    let mut write_ends: Vec<Option<std::fs::File>> = pipes.iter_mut().map(|_| None).collect();
    for (i, (read, write)) in pipes.into_iter().enumerate() {
        read_ends[i] = Some(read);
        write_ends[i] = Some(write);
    }

    let mut handles: Vec<std::thread::JoinHandle<ExecResult>> = Vec::with_capacity(n);

    for i in 0..n {
        let mut stage_env = env.clone();
        stage_env.inherit_traps_for_subshell();
        stage_env.set_bg_pid_reporting_enabled(i == n - 1);
        let stage_source = source.to_string();
        let stage_cmd = commands[i].clone();

        // stdin for this stage: stage 0 inherits, others read from pipe[i-1].
        let stdin_file: Option<std::fs::File> = if i > 0 { read_ends[i - 1].take() } else { None };

        // stdout for this stage: last stage captures (Pipe), others write to pipe[i].
        let stdout_file: Option<std::fs::File> = if i < n - 1 {
            write_ends[i].take()
        } else {
            None
        };

        handles.push(std::thread::spawn(move || {
            execute_with_io(
                &stage_cmd,
                &stage_source,
                &mut stage_env,
                stdin_file,
                stdout_file,
            )
        }));
    }

    // 3. Drop remaining parent-side pipe ends so stages see EOF.
    drop(read_ends);
    drop(write_ends);

    // 4. Join all threads, collect results.
    let results: Vec<ExecResult> = handles
        .into_iter()
        .map(|h| {
            h.join().unwrap_or_else(|e| {
                let msg = if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                tracing::error!(panic = %msg, "pipeline stage panicked");
                ExecResult::with_code(1)
            })
        })
        .collect();

    // 5. Combine output: stdout from last stage, stderr from all stages.
    let mut all_stderr = Vec::new();
    for r in &results {
        all_stderr.extend_from_slice(&r.stderr);
    }
    let last_stdout = results.last().map(|r| r.stdout.clone()).unwrap_or_default();

    // 6. Exit code: pipefail → first nonzero; otherwise → last stage.
    let exit_code = if env.options().pipefail {
        results
            .iter()
            .map(|r| r.exit_code)
            .find(|&c| c != 0)
            .unwrap_or(0)
    } else {
        results.last().map(|r| r.exit_code).unwrap_or(0)
    };

    // Propagate last exit code to the parent env.
    env.set_exit_code(exit_code);

    let mut result = ExecResult {
        exit_code,
        stdout: last_stdout,
        stderr: all_stderr,
    };

    // 7. Negation.
    if negated {
        result.exit_code = if result.exit_code == 0 { 1 } else { 0 };
    }

    result
}

/// Execute a command with externally provided stdin/stdout file handles.
///
/// When `stdin_file` is `Some`, it replaces the command's stdin.
/// When `stdout_file` is `Some`, the command's stdout is written to that
/// file (for external commands the fd is passed directly via `Io::File`;
/// for internal commands the captured output is written manually).
fn execute_with_io(
    cmd: &Spanned<Command>,
    source: &str,
    env: &mut Env,
    stdin_file: Option<std::fs::File>,
    stdout_file: Option<std::fs::File>,
) -> ExecResult {
    // For Simple commands, we intercept the spawn config to wire in the
    // pipe fds directly.  For other command types, we execute normally
    // and then redirect captured output to the pipe.
    match &cmd.node {
        Command::Simple {
            name,
            args,
            redirects,
            env_assigns,
        } => execute_simple_with_io(
            name,
            args,
            redirects,
            env_assigns,
            source,
            env,
            stdin_file,
            stdout_file,
        ),
        Command::Redirected {
            cmd: inner,
            redirects,
        } => {
            if let Command::Simple {
                name,
                args,
                redirects: inner_redirects,
                env_assigns,
            } = &inner.node
            {
                let mut combined_redirects = inner_redirects.clone();
                combined_redirects.extend_from_slice(redirects);
                return execute_simple_with_io(
                    name,
                    args,
                    &combined_redirects,
                    env_assigns,
                    source,
                    env,
                    stdin_file,
                    stdout_file,
                );
            }

            let resolved_io = match resolve_redirects(redirects, source, env) {
                Ok(io) => io,
                Err(err_result) => return err_result,
            };

            let mut result = execute(inner, source, env);

            if let Some(mut pipe_out) = resolved_io.stdout.or(stdout_file) {
                let _ = pipe_out.write_all(&result.stdout);
                result.stdout = Vec::new();
            }
            if let Some(mut pipe_in) = resolved_io.stdin.or(stdin_file) {
                drop(pipe_in);
            }
            if let Some(mut stderr_file) = resolved_io.stderr {
                let _ = stderr_file.write_all(&result.stderr);
                result.stderr = Vec::new();
            }
            result
        }
        _ => match &cmd.node {
            Command::BraceGroup { body } => {
                execute_list_with_io(body, source, env, stdin_file.as_ref(), stdout_file.as_ref())
            }
            Command::Subshell { body } => {
                let mut sub_env = env.clone();
                sub_env.inherit_traps_for_subshell();
                let result = execute_list_with_io(
                    body,
                    source,
                    &mut sub_env,
                    stdin_file.as_ref(),
                    stdout_file.as_ref(),
                );
                env.set_exit_code(result.exit_code);
                result
            }
            _ => {
                let mut result = execute(cmd, source, env);
                drop(stdin_file);
                if let Some(mut pipe_out) = stdout_file {
                    let _ = pipe_out.write_all(&result.stdout);
                    result.stdout = Vec::new();
                }
                result
            }
        },
    }
}

/// Like `execute_simple`, but with externally provided stdin/stdout.
fn execute_simple_with_io(
    name_span: &Span,
    arg_spans: &[Span],
    redirects: &[Spanned<Redirect>],
    env_assigns: &[(Span, Span)],
    source: &str,
    env: &mut Env,
    mut stdin_file: Option<std::fs::File>,
    stdout_file: Option<std::fs::File>,
) -> ExecResult {
    // 1. Expand command name.
    let name_text = name_span.text(source);
    let expanded_name = match expander::expand_word(name_text, env) {
        Ok(fields) => fields,
        Err(e) => return noninteractive_shell_error(env, format!("mash: {e}\n")),
    };
    let cmd_name = match expanded_name.first() {
        Some(n) if !n.is_empty() => n.clone(),
        _ => return ExecResult::success(),
    };
    let dispatch_name = explicit_internal_command_name(&cmd_name);
    let dispatch_name = dispatch_name.as_deref().unwrap_or(&cmd_name);

    // 2. Expand arguments.
    let mut argv: Vec<String> = Vec::new();
    argv.extend(expanded_name.into_iter().skip(1));
    for arg_span in arg_spans {
        let arg_text = arg_span.text(source);
        match expander::expand_word(arg_text, env) {
            Ok(fields) => argv.extend(fields),
            Err(e) => return noninteractive_shell_error(env, format!("mash: {e}\n")),
        }
    }

    // 3. Temporary env assignments.
    let mut child_env: Vec<(String, String)> = Vec::new();
    for (key_span, val_span) in env_assigns {
        let key = key_span.text(source).to_string();
        let val_text = val_span.text(source);
        let val = match expander::expand_assignment_word_nosplit(val_text, env) {
            Ok(v) => v,
            Err(e) => return noninteractive_shell_error(env, format!("mash: {e}\n")),
        };
        child_env.push((key, val));
    }

    // 4. Resolve explicit redirects (these override pipeline I/O).
    let mut resolved_io = match resolve_redirects(redirects, source, env) {
        Ok(io) => io,
        Err(err_result) => return err_result,
    };

    // 5. Handle builtins in pipeline context.
    let builtin_stdin = if BUILTIN_NAMES.contains(&dispatch_name) {
        resolved_io.stdin.take().or(stdin_file.take())
    } else {
        None
    };
    let saved_nonstdio_states = if BUILTIN_NAMES.contains(&dispatch_name) {
        let states: Vec<(u32, SavedFdState)> = nonstdio_affected_fds(&resolved_io)
            .into_iter()
            .map(|fd| (fd, save_fd_state(env, fd)))
            .collect();
        apply_nonstdio_redirects(env, &mut resolved_io);
        states
    } else {
        Vec::new()
    };
    if let Some(mut result) = try_execute_builtin(dispatch_name, &argv, env, builtin_stdin) {
        let builtin_name = builtin_output_name(dispatch_name, &argv);
        apply_builtin_output_redirects(&mut result, resolved_io, &builtin_name);
        for (fd, state) in saved_nonstdio_states {
            restore_fd_state(env, fd, state);
        }
        if let Some(mut pipe_out) = stdout_file {
            if let Err(e) = pipe_out.write_all(&result.stdout) {
                let _ = e;
                return builtin_output_io_error(&builtin_name);
            }
            result.stdout = Vec::new();
        }
        return result;
    }
    for (fd, state) in saved_nonstdio_states {
        restore_fd_state(env, fd, state);
    }

    // 5b. Handle malt-tools (in-process POSIX utilities like grep, cat, wc).
    let tools_registry = malt_tools::Registry::new();
    if dispatch_name == "sleep" && env.current_job_id().is_some() {
        let mut result = execute_interruptible_sleep(&argv, env);
        apply_output_redirects(&mut result, resolved_io);
        if let Some(mut pipe_out) = stdout_file {
            if let Err(e) = pipe_out.write_all(&result.stdout) {
                return ExecResult::failure(1, format!("mash: pipeline write failed: {e}\n"));
            }
            result.stdout = Vec::new();
        }
        return result;
    }
    if tools_registry.contains(dispatch_name) {
        record_hashed_command(env, &cmd_name, dispatch_name);
        let stdin_bytes: Vec<u8> =
            if let Some(mut file) = resolved_io.stdin.take().or(stdin_file.take()) {
                let mut buf = Vec::new();
                if let Err(e) = std::io::Read::read_to_end(&mut file, &mut buf) {
                    return ExecResult::failure(
                        1,
                        format!("mash: {dispatch_name}: read stdin failed: {e}\n"),
                    );
                }
                buf
            } else {
                Vec::new()
            };

        let tool_fn = tools_registry.get(dispatch_name).unwrap();
        let tool_result = tool_fn(&argv, &stdin_bytes);
        let mut result = ExecResult {
            exit_code: tool_result.exit_code,
            stdout: tool_result.stdout,
            stderr: tool_result.stderr,
        };
        apply_output_redirects(&mut result, resolved_io);
        if let Some(mut pipe_out) = stdout_file {
            if let Err(e) = pipe_out.write_all(&result.stdout) {
                return ExecResult::failure(1, format!("mash: pipeline write failed: {e}\n"));
            }
            result.stdout = Vec::new();
        }
        return result;
    }

    // 6. Check for shell functions (in pipeline context).
    if let Some(func_def) = env.get_function(&cmd_name).cloned() {
        if env.call_depth() >= 50 {
            let msg = format!("mash: {cmd_name}: maximum function nesting level exceeded\n");
            return ExecResult::failure(1, msg);
        }
        env.push_scope();
        let saved = env.save_positional();
        env.replace_positional_args(&argv);
        env.push_call(CallFrame {
            name: cmd_name.clone(),
            file: String::new(),
            line: 0,
        });
        for (k, v) in &child_env {
            let _ = env.set(k, Variable::string(v.clone()));
        }
        let stored_source = func_def.source.clone();
        let func_body = func_def.body.clone();
        let mut result = execute(&func_body, &stored_source, env);
        if let LoopControl::Return(code) = env.loop_control().clone() {
            result.exit_code = code;
            env.set_loop_control(LoopControl::None);
        }
        env.pop_call();
        env.restore_positional(saved);
        let _ = env.pop_scope();
        apply_output_redirects(&mut result, resolved_io);
        if let Some(mut pipe_out) = stdout_file {
            let _ = pipe_out.write_all(&result.stdout);
            result.stdout = Vec::new();
        }
        return result;
    }

    // 7. Resolve executable path.
    let program = match find_in_path(&cmd_name, env) {
        Some(p) => p,
        None => {
            let msg = format!("mash: {cmd_name}: command not found\n");
            return ExecResult::failure(127, msg);
        }
    };
    record_hashed_command(env, &cmd_name, &program.to_string_lossy());

    if should_execute_shell_script_with_mash(&program) {
        return execute_shell_script_with_io(
            &cmd_name,
            &program,
            &argv,
            child_env.as_slice(),
            resolved_io,
            env,
            stdin_file,
            stdout_file,
        );
    }

    // 7. Build SpawnConfig with pipeline I/O + redirect overrides.
    let mut config = malt_platform::process::SpawnConfig::new(&program);
    config.args = argv.iter().map(|a| a.into()).collect();
    configure_command_spawn_identity(&mut config, &cmd_name, &program);

    // stdin: explicit redirect wins, then pipeline, then inherit.
    config.stdin = match resolved_io.stdin {
        Some(f) => malt_platform::process::Io::File(f),
        None => match stdin_file {
            Some(f) => malt_platform::process::Io::File(f),
            None => malt_platform::process::Io::Inherit,
        },
    };

    // stdout: explicit redirect wins, then pipeline, then capture (Pipe).
    config.stdout = match resolved_io.stdout {
        Some(f) => malt_platform::process::Io::File(f),
        None => match stdout_file {
            Some(f) => malt_platform::process::Io::File(f),
            None => malt_platform::process::Io::Pipe,
        },
    };

    // stderr: explicit redirect wins, then capture.
    config.stderr = match resolved_io.stderr {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Pipe,
    };

    // Set exported env vars.
    let exported = env.exported_vars();
    for (k, v) in &exported {
        config.env.push((k.into(), v.into()));
    }
    for (k, v) in &child_env {
        config.env.push((k.into(), v.into()));
    }
    config.env_clear = true;

    // Spawn.
    let mut child = match malt_platform::process::spawn(config) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: {e}\n");
            let code = match &e {
                malt_platform::process::SpawnError::NotFound { .. } => 127,
                malt_platform::process::SpawnError::PermissionDenied { .. } => 126,
                _ => 1,
            };
            return ExecResult::failure(code, msg);
        }
    };
    env.report_bg_pid(child.pid());

    // Read captured output.
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        if let Err(e) = out.read_to_end(&mut stdout_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stdout read failed: {e}\n").as_bytes(),
            );
        }
    }
    if let Some(mut err) = child.take_stderr() {
        if let Err(e) = err.read_to_end(&mut stderr_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stderr read failed: {e}\n").as_bytes(),
            );
        }
    }

    // Wait.
    let exit_code = match wait_for_child_exit_code(&mut child, env) {
        Ok(code) => code,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: wait failed: {e}\n");
            stderr_bytes.extend_from_slice(msg.as_bytes());
            1
        }
    };

    ExecResult {
        exit_code,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

// ── Simple command ─────────────────────────────────────────────────────

fn execute_simple(
    name_span: &Span,
    arg_spans: &[Span],
    redirects: &[Spanned<Redirect>],
    env_assigns: &[(Span, Span)],
    source: &str,
    env: &mut Env,
) -> ExecResult {
    // 1. Expand the command name.
    let name_text = name_span.text(source);
    let expanded_name = match expander::expand_word(name_text, env) {
        Ok(fields) => fields,
        Err(e) => return noninteractive_shell_error(env, format!("mash: {e}\n")),
    };
    let cmd_name = match expanded_name.first() {
        Some(n) if !n.is_empty() => n.clone(),
        _ => return ExecResult::success(), // Null command.
    };
    let dispatch_name = explicit_internal_command_name(&cmd_name);
    let dispatch_name = dispatch_name.as_deref().unwrap_or(&cmd_name);

    // 1b. Alias expansion: if the command name matches an alias, substitute and re-execute.
    // Per POSIX: alias expansion happens early, before other lookups.
    if let Some(alias_val) = env.get_alias(&cmd_name).map(|s| s.to_string()) {
        // Prevent infinite self-recursion: if the alias value's first word
        // equals the alias name, skip expansion.
        let first_word = alias_val.split_whitespace().next().unwrap_or("");
        if first_word != cmd_name {
            // Build substituted command: alias value + original args + redirects
            let mut substituted = alias_val;
            for arg_span in arg_spans {
                substituted.push(' ');
                substituted.push_str(arg_span.text(source));
            }
            for r in redirects {
                substituted.push(' ');
                substituted.push_str(&redirect_to_text(r, source));
            }

            // An empty alias (e.g., `alias empty=''`) with no args/redirects
            // is a null command — exit 0.
            if substituted.trim().is_empty() {
                return ExecResult::success();
            }

            // Re-parse and execute the substituted command.
            match crate::parser::parse(&substituted) {
                Ok(commands) if !commands.is_empty() => {
                    let mut result = ExecResult::success();
                    for cmd in &commands {
                        result = execute(cmd, source, env);
                    }
                    return result;
                }
                _ => {
                    // Parse failed — fall through to normal command lookup.
                }
            }
        }
    }

    // 2. Expand all arguments.
    let mut argv: Vec<String> = Vec::new();
    // Include remaining fields from name expansion (if word split produced multiple).
    argv.extend(expanded_name.into_iter().skip(1));
    for arg_span in arg_spans {
        let arg_text = arg_span.text(source);
        match expander::expand_word(arg_text, env) {
            Ok(fields) => argv.extend(fields),
            Err(e) => return noninteractive_shell_error(env, format!("mash: {e}\n")),
        }
    }

    // 3. Resolve redirects before prefix assignment expansion so redirect-side
    // effects are visible first, matching shell prefix evaluation order.
    let mut resolved_io = match resolve_redirects(redirects, source, env) {
        Ok(io) => io,
        Err(err_result) => {
            if redirect_error_aborts_noninteractive_shell(env, &cmd_name, &err_result) {
                env.request_exit(err_result.exit_code);
            }
            return err_result;
        }
    };

    // 4. Expand prefix assignments left-to-right in a temporary shell view so
    // earlier assignments are visible to later ones.
    let child_env = match expand_prefix_assignments(env_assigns, source, env) {
        Ok(assignments) => assignments,
        Err(err) => return err,
    };

    // 5. Handle special builtins (break, continue, return, true, false, exit, :, echo).
    // Note: Don't take stdin here - only take it if the builtin actually needs it.
    if is_special_builtin_name(dispatch_name) {
        for (k, v) in &child_env {
            let _ = env.set(k, Variable::string(v.clone()));
        }
    }
    if BUILTIN_NAMES.contains(&dispatch_name) {
        let persist_nonstdio_redirects = is_exec_no_args_command(&cmd_name, &argv)
            || (dispatch_name == "command"
                && argv.len() == 1
                && argv.first().map(|s| s.as_str()) == Some("exec"));
        let saved_nonstdio_states = if persist_nonstdio_redirects {
            Vec::new()
        } else {
            nonstdio_affected_fds(&resolved_io)
                .into_iter()
                .map(|fd| (fd, save_fd_state(env, fd)))
                .collect()
        };
        apply_nonstdio_redirects(env, &mut resolved_io);
        let builtin_stdin = if is_exec_no_args_command(&cmd_name, &argv) {
            None
        } else {
            resolved_io.stdin.take()
        };
        if let Some(mut result) = try_execute_builtin(dispatch_name, &argv, env, builtin_stdin) {
            if is_exec_no_args_command(&cmd_name, &argv) {
                apply_exec_redirects(env, &mut resolved_io);
            }
            let builtin_name = builtin_output_name(dispatch_name, &argv);
            apply_builtin_output_redirects(&mut result, resolved_io, &builtin_name);
            if !persist_nonstdio_redirects {
                for (fd, state) in saved_nonstdio_states {
                    restore_fd_state(env, fd, state);
                }
            }
            return result;
        }
        for (fd, state) in saved_nonstdio_states {
            restore_fd_state(env, fd, state);
        }
    }

    if cmd_name == "exec" && !argv.is_empty() {
        let mut result =
            execute_expanded_command(&argv[0], &argv[1..], child_env.as_slice(), resolved_io, env);
        env.request_exit(result.exit_code);
        return result;
    }

    // 5b. Handle malt-tools (in-process POSIX utilities like grep, cat, wc, env, which).
    let tools_registry = malt_tools::Registry::new();
    if dispatch_name == "sleep" && env.current_job_id().is_some() {
        let mut result = execute_interruptible_sleep(&argv, env);
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }
    if tools_registry.contains(dispatch_name) {
        record_hashed_command(env, &cmd_name, dispatch_name);
        // Read stdin if redirected.
        let stdin_bytes: Vec<u8> = if let Some(mut file) = resolved_io.stdin.take() {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut file, &mut buf);
            buf
        } else if let Ok(mut file) = env.open_fd_read(0) {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut file, &mut buf);
            buf
        } else {
            Vec::new()
        };

        // Execute the tool.
        let tool_fn = tools_registry.get(dispatch_name).unwrap();
        let tool_result = tool_fn(&argv, &stdin_bytes);

        // Convert to ExecResult and apply redirects.
        let mut result = ExecResult {
            exit_code: tool_result.exit_code,
            stdout: tool_result.stdout,
            stderr: tool_result.stderr,
        };
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }

    // 6. Check for shell functions.
    if let Some(func_def) = env.get_function(&cmd_name).cloned() {
        // Check call depth limit.
        if env.call_depth() >= 50 {
            let msg = format!("mash: {cmd_name}: maximum function nesting level exceeded\n");
            return ExecResult::failure(1, msg);
        }

        // Push scope, save/set positional params.
        env.push_scope();
        let saved = env.save_positional();
        env.replace_positional_args(&argv);
        env.push_call(CallFrame {
            name: cmd_name.clone(),
            file: String::new(),
            line: 0,
        });

        // Handle loop depth: in lexical mode, reset to 0; in non-lexical, preserve it.
        let saved_loop_depth = if env.options().nonlexicalctrl {
            env.loop_depth()
        } else {
            let prev = env.loop_depth();
            env.set_loop_depth(0);
            prev
        };

        // Apply temporary env assignments in function scope.
        for (k, v) in &child_env {
            let _ = env.set(k, Variable::string(v.clone()));
        }

        // Execute function body using the stored source (spans reference it).
        let stored_source = func_def.source.clone();
        let func_body = func_def.body.clone();
        let mut result = execute(&func_body, &stored_source, env);

        // Handle loop control at function boundary.
        match env.loop_control().clone() {
            LoopControl::Return(code) => {
                result.exit_code = code;
                env.set_loop_control(LoopControl::None);
            }
            LoopControl::Break(_) | LoopControl::Continue(_) => {
                if !env.options().nonlexicalctrl {
                    // Lexical mode: consume break/continue (can't escape function)
                    env.set_loop_control(LoopControl::None);
                }
                // Non-lexical mode: leave it set for enclosing loop to handle
            }
            LoopControl::None => {}
        }

        // Restore loop depth.
        env.set_loop_depth(saved_loop_depth);

        // Restore state.
        env.pop_call();
        env.restore_positional(saved);
        let _ = env.pop_scope();

        // Apply redirects to function output.
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }

    // 6. Resolve the executable path.
    let program = match find_in_path(&cmd_name, env) {
        Some(p) => p,
        None => {
            let msg = format!("mash: {cmd_name}: command not found\n");
            return ExecResult::failure(127, msg);
        }
    };
    record_hashed_command(env, &cmd_name, &program.to_string_lossy());

    if should_execute_shell_script_with_mash(&program) {
        return execute_shell_script_with_io(
            &cmd_name,
            &program,
            &argv,
            child_env.as_slice(),
            resolved_io,
            env,
            None,
            None,
        );
    }

    // 7. Build SpawnConfig and execute.
    let mut config = malt_platform::process::SpawnConfig::new(&program);
    config.args = argv.iter().map(|a| a.into()).collect();
    configure_command_spawn_identity(&mut config, &cmd_name, &program);

    // Apply redirect files to the spawn config.
    config.stdin = match resolved_io.stdin {
        Some(f) => malt_platform::process::Io::File(f),
        None => match env.open_fd_read(0) {
            Ok(f) => malt_platform::process::Io::File(f),
            Err(_) => malt_platform::process::Io::Inherit,
        },
    };
    config.stdout = match resolved_io.stdout {
        Some(f) => malt_platform::process::Io::File(f),
        None if env.fd_snapshot_path(1).is_none() && env.has_fd(1) => match env.open_fd_write(1) {
            Ok(f) => malt_platform::process::Io::File(f),
            Err(_) => malt_platform::process::Io::Pipe, // Capture for ExecResult.
        },
        None => malt_platform::process::Io::Pipe, // Capture for ExecResult.
    };
    config.stderr = match resolved_io.stderr {
        Some(f) => malt_platform::process::Io::File(f),
        None if env.fd_snapshot_path(2).is_none() && env.has_fd(2) => match env.open_fd_write(2) {
            Ok(f) => malt_platform::process::Io::File(f),
            Err(_) => malt_platform::process::Io::Pipe, // Capture for ExecResult.
        },
        None => malt_platform::process::Io::Pipe, // Capture for ExecResult.
    };

    // Set exported env vars from the shell environment.
    let exported = env.exported_vars();
    for (k, v) in &exported {
        config.env.push((k.into(), v.into()));
    }
    // Apply temporary assignments (override exported vars).
    for (k, v) in &child_env {
        config.env.push((k.into(), v.into()));
    }
    // Clear the child's inherited env so we control exactly what's passed.
    config.env_clear = true;
    add_runtime_spawn_env(&mut config, env);

    // Spawn the process.
    let mut child = match malt_platform::process::spawn(config) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: {e}\n");
            let code = match &e {
                malt_platform::process::SpawnError::NotFound { .. } => 127,
                malt_platform::process::SpawnError::PermissionDenied { .. } => 126,
                _ => 1,
            };
            return ExecResult::failure(code, msg);
        }
    };
    env.report_bg_pid(child.pid());

    // Read stdout and stderr (only populated when fd is Pipe, not File).
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        if let Err(e) = out.read_to_end(&mut stdout_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stdout read failed: {e}\n").as_bytes(),
            );
        }
    }
    if let Some(mut err) = child.take_stderr() {
        if let Err(e) = err.read_to_end(&mut stderr_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stderr read failed: {e}\n").as_bytes(),
            );
        }
    }

    // Wait for the child.
    let exit_code = match wait_for_child_exit_code(&mut child, env) {
        Ok(code) => code,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: wait failed: {e}\n");
            stderr_bytes.extend_from_slice(msg.as_bytes());
            1
        }
    };

    ExecResult {
        exit_code,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

fn expand_prefix_assignments(
    env_assigns: &[(Span, Span)],
    source: &str,
    env: &mut Env,
) -> Result<Vec<(String, String)>, ExecResult> {
    let mut assign_env = env.clone();
    let mut child_env = Vec::new();

    for (key_span, val_span) in env_assigns {
        let key = key_span.text(source).to_string();
        let val_text = val_span.text(source);
        let val = match expander::expand_assignment_word_nosplit(val_text, &mut assign_env) {
            Ok(v) => v,
            Err(e) => return Err(noninteractive_shell_error(env, format!("mash: {e}\n"))),
        };
        let _ = assign_env.set(&key, Variable::string(val.clone()));
        child_env.push((key, val));
    }

    Ok(child_env)
}

// ── Builtin commands (minimal set for control flow) ───────────────────

/// Try to execute a builtin command. Returns None if not a builtin.
fn try_execute_builtin(
    cmd_name: &str,
    argv: &[String],
    env: &mut Env,
    stdin_file: Option<std::fs::File>,
) -> Option<ExecResult> {
    match cmd_name {
        "break" => {
            let requested: usize = argv
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1)
                .max(1);
            let n = if env.options().nonlexicalctrl {
                requested
            } else {
                let current_depth = env.loop_depth();
                if current_depth == 0 {
                    return Some(ExecResult::failure(
                        1,
                        format!("mash: break: {}: loop level out of range\n", requested),
                    ));
                }
                requested.min(current_depth)
            };
            env.set_loop_control(LoopControl::Break(n));
            Some(ExecResult::success())
        }
        "continue" => {
            let requested: usize = argv
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1)
                .max(1);
            let n = if env.options().nonlexicalctrl {
                requested
            } else {
                let current_depth = env.loop_depth();
                if current_depth == 0 {
                    return Some(ExecResult::failure(
                        1,
                        format!("mash: continue: {}: loop level out of range\n", requested),
                    ));
                }
                requested.min(current_depth)
            };
            env.set_loop_control(LoopControl::Continue(n));
            Some(ExecResult::success())
        }
        "return" => {
            let code: i32 = argv
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(env.exit_code());
            env.set_loop_control(LoopControl::Return(code));
            Some(ExecResult::with_code(code))
        }
        "exit" => {
            let code: i32 = argv
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(env.exit_code());
            env.request_exit(code);
            Some(ExecResult::with_code(code))
        }
        "true" | ":" => Some(ExecResult::success()),
        "false" => Some(ExecResult::with_code(1)),
        "exec" => {
            // exec with no args: return success (redirects are handled by caller).
            // exec with args: execute command and replace shell (simulate by executing and exiting).
            if argv.is_empty() {
                // Just applying redirects in the current shell.
                Some(ExecResult::success())
            } else {
                // For exec with args, we fall through to normal execution,
                // but we need to ensure the shell exits after.
                // The caller (execute_simple_with_io) will handle this because
                // we're in try_execute_builtin and returning None falls through.
                None
            }
        }
        "shopt" => Some(builtin_shopt(&argv, env)),
        "times" => {
            // Print accumulated CPU times for shell and children.
            match malt_platform::resource::get_rusage() {
                Ok(usage) => {
                    let fmt = |secs: f64| -> String {
                        let mins = (secs / 60.0) as u64;
                        let rem = secs - (mins as f64 * 60.0);
                        format!("{}m{:.3}s", mins, rem)
                    };
                    let out = format!(
                        "{} {}\n{} {}\n",
                        fmt(usage.user_secs),
                        fmt(usage.sys_secs),
                        fmt(usage.child_user_secs),
                        fmt(usage.child_sys_secs),
                    );
                    Some(ExecResult {
                        exit_code: 0,
                        stdout: out.into_bytes(),
                        stderr: Vec::new(),
                    })
                }
                Err(_) => {
                    // Fallback to zeros when unavailable (Windows, or errors)
                    let out = "0m0.000s 0m0.000s\n0m0.000s 0m0.000s\n";
                    Some(ExecResult {
                        exit_code: 0,
                        stdout: out.as_bytes().to_vec(),
                        stderr: Vec::new(),
                    })
                }
            }
        }
        "echo" => {
            // POSIX leaves echo option handling implementation-defined.
            // Support the common `-n` form used by the conformance suite.
            let mut suppress_newline = false;
            let mut arg_index = 0;
            if argv.first().is_some_and(|arg| arg == "-n") {
                suppress_newline = true;
                arg_index = 1;
            }

            let mut output = argv[arg_index..].join(" ");
            if !suppress_newline {
                output.push('\n');
            }
            Some(ExecResult {
                exit_code: 0,
                stdout: output.into_bytes(),
                stderr: Vec::new(),
            })
        }
        "local" => {
            // Basic local: set variables in current scope.
            for arg in argv {
                if let Some((name, val)) = arg.split_once('=') {
                    let _ = env.set(name, Variable::string(val));
                } else {
                    // Declare without value.
                    if env.get(arg).is_none() {
                        let _ = env.set(arg, Variable::string(""));
                    }
                }
            }
            Some(ExecResult::success())
        }
        "eval" => {
            // Concatenate args and re-parse/execute.
            let input = argv.join(" ");
            if input.is_empty() {
                return Some(ExecResult::success());
            }
            match crate::parser::parse(&input) {
                Ok(cmds) => {
                    let result = execute_list(&cmds, &input, env);
                    Some(result)
                }
                Err(e) => {
                    if !env.is_interactive() {
                        env.request_exit(1);
                    }
                    Some(ExecResult::failure(1, format!("mash: eval: {e}\n")))
                }
            }
        }
        "set" => {
            // Handle set options and positional parameters.
            let mut i = 0;
            while i < argv.len() {
                let arg = &argv[i];
                if arg == "--" {
                    // Everything after -- becomes positional parameters.
                    let args: Vec<String> = argv[i + 1..].to_vec();
                    env.replace_positional_args(&args);
                    return Some(ExecResult::success());
                } else if arg.starts_with('-') && arg.len() > 1 {
                    for flag in arg[1..].chars() {
                        match flag {
                            'e' => env.options_mut().errexit = true,
                            'u' => env.options_mut().nounset = true,
                            'x' => env.options_mut().xtrace = true,
                            'v' => env.options_mut().verbose = true,
                            'f' => env.options_mut().noglob = true,
                            'b' => env.options_mut().notify = true,
                            'm' => env.options_mut().monitor = true,
                            'C' => env.options_mut().noclobber = true,
                            'n' => env.options_mut().noexec = true,
                            'h' => env.options_mut().hash_cmds = true,
                            'o' => {
                                // set -o pipefail, etc.
                                if i + 1 < argv.len() {
                                    i += 1;
                                    match argv[i].as_str() {
                                        "errexit" => env.options_mut().errexit = true,
                                        "nounset" => env.options_mut().nounset = true,
                                        "xtrace" => env.options_mut().xtrace = true,
                                        "verbose" => env.options_mut().verbose = true,
                                        "noglob" => env.options_mut().noglob = true,
                                        "pipefail" => env.options_mut().pipefail = true,
                                        "nolog" => env.options_mut().nolog = true,
                                        "noclobber" => env.options_mut().noclobber = true,
                                        "noexec" => env.options_mut().noexec = true,
                                        "nonlexicalctrl" => env.options_mut().nonlexicalctrl = true,
                                        invalid => {
                                            return Some(ExecResult::failure(
                                                1,
                                                format!("mash: set: {invalid}: invalid option\n"),
                                            ));
                                        }
                                    }
                                } else {
                                    return Some(ExecResult::failure(
                                        1,
                                        "mash: set: -o: option name required\n",
                                    ));
                                }
                            }
                            invalid => {
                                return Some(ExecResult::failure(
                                    1,
                                    format!("mash: set: -{invalid}: invalid option\n"),
                                ));
                            }
                        }
                    }
                } else if arg.starts_with('+') && arg.len() > 1 {
                    for flag in arg[1..].chars() {
                        match flag {
                            'e' => env.options_mut().errexit = false,
                            'u' => env.options_mut().nounset = false,
                            'x' => env.options_mut().xtrace = false,
                            'v' => env.options_mut().verbose = false,
                            'f' => env.options_mut().noglob = false,
                            'b' => env.options_mut().notify = false,
                            'm' => env.options_mut().monitor = false,
                            'C' => env.options_mut().noclobber = false,
                            'n' => env.options_mut().noexec = false,
                            'h' => env.options_mut().hash_cmds = false,
                            'o' => {
                                if i + 1 < argv.len() {
                                    i += 1;
                                    match argv[i].as_str() {
                                        "errexit" => env.options_mut().errexit = false,
                                        "nounset" => env.options_mut().nounset = false,
                                        "xtrace" => env.options_mut().xtrace = false,
                                        "verbose" => env.options_mut().verbose = false,
                                        "noglob" => env.options_mut().noglob = false,
                                        "pipefail" => env.options_mut().pipefail = false,
                                        "nolog" => env.options_mut().nolog = false,
                                        "noclobber" => env.options_mut().noclobber = false,
                                        "noexec" => env.options_mut().noexec = false,
                                        "nonlexicalctrl" => {
                                            env.options_mut().nonlexicalctrl = false
                                        }
                                        invalid => {
                                            return Some(ExecResult::failure(
                                                1,
                                                format!("mash: set: {invalid}: invalid option\n"),
                                            ));
                                        }
                                    }
                                } else {
                                    return Some(ExecResult::failure(
                                        1,
                                        "mash: set: +o: option name required\n",
                                    ));
                                }
                            }
                            invalid => {
                                return Some(ExecResult::failure(
                                    1,
                                    format!("mash: set: +{invalid}: invalid option\n"),
                                ));
                            }
                        }
                    }
                } else {
                    // Bare args become positional parameters.
                    let args: Vec<String> = argv[i..].to_vec();
                    env.replace_positional_args(&args);
                    return Some(ExecResult::success());
                }
                i += 1;
            }
            Some(ExecResult::success())
        }
        "shift" => {
            let n: usize = argv.first().and_then(|s| s.parse().ok()).unwrap_or(1);
            let current_count: usize = env.get_str("#").parse().unwrap_or(0);
            if n > current_count {
                return Some(ExecResult::with_code(1));
            }
            let args: Vec<String> = (n + 1..=current_count)
                .map(|i| env.get_str(&i.to_string()).to_string())
                .collect();
            env.replace_positional_args(&args);
            Some(ExecResult::success())
        }
        "source" | "." => {
            let builtin_label = if cmd_name == "." { "." } else { "source" };
            let file = match argv.first() {
                Some(f) => f,
                None => return Some(ExecResult::with_code(2)),
            };
            let path = match resolve_source_path(file, env) {
                Some(path) => path,
                None => {
                    if !env.is_interactive() {
                        env.request_exit(1);
                    }
                    return Some(ExecResult {
                        exit_code: 1,
                        stdout: Vec::new(),
                        stderr: format!("{builtin_label}: {}: not found\n", file).into_bytes(),
                    });
                }
            };
            if !malt_platform::fs::is_readable(&path) {
                if !env.is_interactive() {
                    env.request_exit(1);
                }
                return Some(ExecResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: format!("{builtin_label}: {}: permission denied\n", path.display())
                        .into_bytes(),
                });
            }
            let contents = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    if !env.is_interactive() {
                        env.request_exit(1);
                    }
                    return Some(ExecResult {
                        exit_code: 1,
                        stdout: Vec::new(),
                        stderr: format!("{builtin_label}: {}: {}\n", path.display(), e).into_bytes(),
                    });
                }
            };
            match crate::parser::parse(&contents) {
                Ok(cmds) => {
                    let mut result = execute_list(&cmds, &contents, env);
                    // Handle loop control and return propagation from sourced script
                    match env.loop_control().clone() {
                        LoopControl::Return(code) => {
                            result.exit_code = code;
                            env.set_loop_control(LoopControl::None);
                        }
                        LoopControl::Break(_) | LoopControl::Continue(_) => {
                            // Source propagates break/continue only in non-lexical mode
                            if !env.options().nonlexicalctrl {
                                // Lexical mode: consume break/continue (can't escape sourced script)
                                env.set_loop_control(LoopControl::None);
                            }
                            // Non-lexical mode: leave it set for enclosing loop to handle
                        }
                        LoopControl::None => {}
                    }
                    Some(result)
                }
                Err(e) => Some(ExecResult {
                    exit_code: {
                        if !env.is_interactive() {
                            env.request_exit(1);
                        }
                        1
                    },
                    stdout: Vec::new(),
                    stderr: format!("{builtin_label}: parse error: {}\n", e).into_bytes(),
                }),
            }
        }
        "export" => {
            // -p or no args: list all exported variables.
            let print_only = argv.is_empty() || (argv.len() == 1 && argv[0] == "-p");
            if print_only {
                let exported = env.exported_vars();
                let mut lines: Vec<String> = exported
                    .iter()
                    .map(|(k, v)| {
                        if v.is_empty() {
                            format!("export {}\n", k)
                        } else {
                            format!("export {}=\"{}\"\n", k, v)
                        }
                    })
                    .collect();
                lines.sort_unstable();
                return Some(ExecResult {
                    exit_code: 0,
                    stdout: lines.concat().into_bytes(),
                    stderr: Vec::new(),
                });
            }

            let mut exit_code = 0;
            let mut errors = Vec::new();
            for (i, arg) in argv.iter().enumerate() {
                if arg == "-p" || arg == "-n" {
                    continue;
                }
                if let Some((name, val)) = arg.split_once('=') {
                    if env.get(name).is_some_and(|v| v.readonly) {
                        errors.extend_from_slice(
                            format!("mash: export: {}: readonly variable\n", name).as_bytes(),
                        );
                        if !env.is_interactive() {
                            env.request_exit(1);
                        }
                        exit_code = 1;
                        continue;
                    }
                    let var = Variable::exported_string(val);
                    let _ = env.set(name, var);
                } else {
                    // Check if previous arg was -n (un-export).
                    if i > 0 && argv[i - 1] == "-n" {
                        env.mark_unexported(arg);
                    } else if env.get(arg).is_some() {
                        env.mark_exported(arg);
                    } else {
                        // Export unset variable with empty value.
                        let _ = env.set(arg, Variable::exported_string(""));
                    }
                }
            }

            Some(ExecResult {
                exit_code,
                stdout: Vec::new(),
                stderr: errors,
            })
        }
        "unset" => {
            let mut unset_func = false;
            let mut names = &argv[..];
            if let Some(first) = argv.first() {
                if first == "-f" {
                    unset_func = true;
                    names = &argv[1..];
                } else if first == "-v" {
                    names = &argv[1..];
                }
            }

            let mut exit_code = 0;
            let mut errors = Vec::new();
            for name in names {
                if unset_func {
                    env.unset_function(name);
                } else {
                    match env.unset(name) {
                        Ok(_) => {}
                        Err(e) => {
                            let msg = match e {
                                EnvError::ReadonlyVariable(var) => {
                                    format!("unset: {var} is read-only\n")
                                }
                                _ => format!("mash: unset: {e}\n"),
                            };
                            errors.extend_from_slice(msg.as_bytes());
                            if !env.is_interactive() {
                                env.request_exit(1);
                            }
                            exit_code = 1;
                        }
                    }
                }
            }

            Some(ExecResult {
                exit_code,
                stdout: Vec::new(),
                stderr: errors,
            })
        }
        "readonly" => {
            // -p or no args: list all readonly variables.
            let print_only = argv.is_empty() || (argv.len() == 1 && argv[0] == "-p");
            if print_only {
                let readonly = env.readonly_vars();
                let mut lines: Vec<String> = readonly
                    .iter()
                    .map(|(k, v)| format!("declare -r {}=\"{}\"\n", k, v))
                    .collect();
                lines.sort_unstable();
                return Some(ExecResult {
                    exit_code: 0,
                    stdout: lines.concat().into_bytes(),
                    stderr: Vec::new(),
                });
            }

            let mut exit_code = 0;
            let mut errors = Vec::new();
            for arg in argv {
                if arg == "-p" || arg == "--" {
                    continue;
                }
                if let Some((name, val)) = arg.split_once('=') {
                    if env.get(name).is_some_and(|v| v.readonly) {
                        errors.extend_from_slice(
                            format!("readonly: {}: is read only\n", name).as_bytes(),
                        );
                        if !env.is_interactive() {
                            env.request_exit(1);
                        }
                        exit_code = 1;
                        continue;
                    }
                    let mut var = Variable::string(val);
                    var.readonly = true;
                    let _ = env.set(name, var);
                } else {
                    env.mark_readonly(arg);
                }
            }

            Some(ExecResult {
                exit_code,
                stdout: Vec::new(),
                stderr: errors,
            })
        }
        "cd" => {
            let target = match argv.first().map(|s| s.as_str()) {
                Some("-") => {
                    let oldpwd = env.get_str("OLDPWD").to_string();
                    if oldpwd.is_empty() {
                        return Some(ExecResult::failure(1, "mash: cd: OLDPWD not set\n"));
                    }
                    oldpwd
                }
                Some(dir) => dir.to_string(),
                None => {
                    let home = env.get_str("HOME").to_string();
                    if home.is_empty() {
                        return Some(ExecResult::failure(1, "mash: cd: HOME not set\n"));
                    }
                    home
                }
            };

            let old_pwd = env.get_str("PWD").to_string();

            match std::env::set_current_dir(&target) {
                Ok(()) => {
                    let new_pwd = std::env::current_dir()
                        .map(|p| {
                            let s = p.to_string_lossy().into_owned();
                            #[cfg(windows)]
                            {
                                s.replace('\\', "/")
                            }
                            #[cfg(not(windows))]
                            {
                                s
                            }
                        })
                        .unwrap_or_else(|_| target.clone());

                    let _ = env.set("OLDPWD", Variable::exported_string(&old_pwd));
                    let _ = env.set("PWD", Variable::exported_string(&new_pwd));

                    // cd - prints the new directory
                    let print_dir = argv.first().map(|s| s.as_str()) == Some("-");
                    if print_dir {
                        Some(ExecResult {
                            exit_code: 0,
                            stdout: format!("{}\n", new_pwd).into_bytes(),
                            stderr: Vec::new(),
                        })
                    } else {
                        Some(ExecResult::success())
                    }
                }
                Err(e) => Some(ExecResult::failure(
                    1,
                    format!("mash: cd: {}: {}\n", target, e),
                )),
            }
        }
        "pwd" => {
            let physical = argv.iter().any(|a| a == "-P");
            let dir = if physical {
                // -P: resolve symlinks via current_dir
                std::env::current_dir()
                    .map(|p| {
                        let s = p.to_string_lossy().into_owned();
                        #[cfg(windows)]
                        {
                            s.replace('\\', "/")
                        }
                        #[cfg(not(windows))]
                        {
                            s
                        }
                    })
                    .unwrap_or_default()
            } else {
                // Logical: prefer $PWD, fall back to current_dir
                let pwd = env.get_str("PWD").to_string();
                if pwd.is_empty() {
                    std::env::current_dir()
                        .map(|p| {
                            let s = p.to_string_lossy().into_owned();
                            #[cfg(windows)]
                            {
                                s.replace('\\', "/")
                            }
                            #[cfg(not(windows))]
                            {
                                s
                            }
                        })
                        .unwrap_or_default()
                } else {
                    pwd
                }
            };

            Some(ExecResult {
                exit_code: 0,
                stdout: format!("{}\n", dir).into_bytes(),
                stderr: Vec::new(),
            })
        }

        // ── test / [ ─────────────────────────────────────────────────
        "test" => Some(builtin_test(argv, false)),
        "[" => {
            // `[` requires a closing `]` as the last argument.
            if argv.last().map(|s| s.as_str()) != Some("]") {
                return Some(ExecResult::failure(2, "mash: [: missing `]'\n"));
            }
            let inner = &argv[..argv.len() - 1];
            Some(builtin_test(inner, true))
        }

        // ── trap ─────────────────────────────────────────────────────
        "trap" => Some(builtin_trap(argv, env)),

        // ── history / jobs / kill / bg / fg ────────────────────────
        "history" => Some(builtin_history(argv, env)),
        "jobs" => Some(builtin_jobs(argv, env)),
        "kill" => Some(builtin_kill(argv, env)),
        "wait" => Some(builtin_wait(argv, env)),
        "bg" => Some(builtin_bg(argv, env)),
        "fg" => Some(builtin_fg(argv, env)),

        // ── type ─────────────────────────────────────────────────────
        "type" => Some(builtin_type(argv, env)),

        // ── hash ─────────────────────────────────────────────────────
        "hash" => Some(builtin_hash(argv, env)),

        // ── command ──────────────────────────────────────────────────
        "command" => Some(builtin_command(argv, env, stdin_file)),

        // ── read ─────────────────────────────────────────────────────
        "read" => Some(builtin_read(argv, env, stdin_file)),

        // ── printf ───────────────────────────────────────────────────
        "printf" => Some(builtin_printf(argv)),

        // ── alias ────────────────────────────────────────────────────
        "alias" => Some(builtin_alias(argv, env)),

        // ── unalias ──────────────────────────────────────────────────
        "unalias" => Some(builtin_unalias(argv, env)),

        // ── getopts ─────────────────────────────────────────────────
        "getopts" => Some(builtin_getopts(argv, env)),

        // ── umask ────────────────────────────────────────────────────
        "umask" => Some(builtin_umask(argv)),

        _ => None,
    }
}

// ── test / [ implementation ──────────────────────────────────────────

/// POSIX test builtin — recursive descent on args slice.
fn builtin_test(args: &[String], _bracket: bool) -> ExecResult {
    if args.is_empty() {
        return ExecResult::with_code(1); // test with no args is false
    }
    match test_evaluate(args) {
        Ok(true) => ExecResult::with_code(0),
        Ok(false) => ExecResult::with_code(1),
        Err(msg) => ExecResult::failure(2, format!("mash: test: {msg}\n")),
    }
}

/// Top-level evaluation: handles -o (OR) at lowest precedence.
fn test_evaluate(args: &[String]) -> Result<bool, String> {
    let mut pos = 0;
    let result = test_eval_or(args, &mut pos, args.len())?;
    if pos != args.len() {
        return Err(format!("unexpected argument: `{}`", args[pos]));
    }
    Ok(result)
}

fn test_eval_or(args: &[String], pos: &mut usize, end: usize) -> Result<bool, String> {
    let mut result = test_eval_and(args, pos, end)?;
    while *pos < end && args[*pos] == "-o" {
        *pos += 1;
        let right = test_eval_and(args, pos, end)?;
        result = result || right;
    }
    Ok(result)
}

fn test_eval_and(args: &[String], pos: &mut usize, end: usize) -> Result<bool, String> {
    let mut result = test_eval_not(args, pos, end)?;
    while *pos < end && args[*pos] == "-a" {
        *pos += 1;
        let right = test_eval_not(args, pos, end)?;
        result = result && right;
    }
    Ok(result)
}

fn test_eval_not(args: &[String], pos: &mut usize, end: usize) -> Result<bool, String> {
    if *pos < end && args[*pos] == "!" {
        *pos += 1;
        let val = test_eval_not(args, pos, end)?;
        Ok(!val)
    } else {
        test_eval_primary(args, pos, end)
    }
}

fn test_eval_primary(args: &[String], pos: &mut usize, end: usize) -> Result<bool, String> {
    if *pos >= end {
        return Ok(false);
    }

    // Look ahead for binary operators
    if *pos + 2 <= end {
        if *pos + 1 < end {
            let maybe_op = args[*pos + 1].as_str();
            match maybe_op {
                "=" | "!=" | "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" | "-ef" | "-nt"
                | "-ot" => {
                    let left = &args[*pos];
                    *pos += 2; // skip left and operator
                    if *pos >= end {
                        return Err(format!("expected operand after `{maybe_op}'"));
                    }
                    let right = &args[*pos];
                    *pos += 1;
                    return test_binary(left, maybe_op, right);
                }
                _ => {}
            }
        }
    }

    // Parenthesized expression
    if args[*pos] == "(" {
        *pos += 1;
        let val = test_eval_or(args, pos, end)?;
        if *pos >= end || args[*pos] != ")" {
            return Err("missing `)'".to_string());
        }
        *pos += 1;
        return Ok(val);
    }

    let arg = &args[*pos];

    // Unary operators
    if arg.starts_with('-') && arg.len() == 2 {
        let op = arg.as_str();
        match op {
            "-z" | "-n" | "-e" | "-f" | "-d" | "-r" | "-w" | "-x" | "-s" | "-L" | "-h" | "-b"
            | "-c" | "-p" | "-S" | "-g" | "-u" | "-k" | "-t" => {
                if *pos + 1 >= end {
                    // Single-arg form: `-z` with no operand? Treat the operator as a non-empty string.
                    *pos += 1;
                    return Ok(true); // the string "-z" is non-empty
                }
                *pos += 1;
                let operand = &args[*pos];
                *pos += 1;
                return test_unary(op, operand);
            }
            _ => {}
        }
    }

    // Single string: true if non-empty
    *pos += 1;
    Ok(!arg.is_empty())
}

fn test_unary(op: &str, operand: &str) -> Result<bool, String> {
    use std::path::Path;
    let path = Path::new(operand);
    match op {
        "-z" => Ok(operand.is_empty()),
        "-n" => Ok(!operand.is_empty()),
        "-e" => Ok(path.exists()),
        "-f" => Ok(path.is_file()),
        "-d" => Ok(path.is_dir()),
        "-s" => Ok(path.metadata().map(|m| m.len() > 0).unwrap_or(false)),
        "-r" => Ok(malt_platform::fs::is_readable(path)),
        "-w" => Ok(malt_platform::fs::is_writable(path)),
        "-x" => Ok(malt_platform::io::is_executable(path)),
        "-L" | "-h" => Ok(path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)),
        "-b" | "-c" | "-p" | "-S" | "-g" | "-u" | "-k" => {
            // Block/char device, pipe, socket, setgid, setuid, sticky —
            // not commonly available on all platforms; return false.
            Ok(false)
        }
        "-t" => {
            // -t fd: is fd a terminal
            if let Ok(fd) = operand.parse::<i32>() {
                Ok(malt_platform::io::is_tty(fd))
            } else {
                Err(format!("integer expression expected: `{operand}'"))
            }
        }
        _ => Err(format!("unknown unary operator: `{op}'")),
    }
}

fn test_binary(left: &str, op: &str, right: &str) -> Result<bool, String> {
    match op {
        "=" => Ok(left == right),
        "!=" => Ok(left != right),
        "-ef" => {
            let left = std::path::Path::new(left)
                .canonicalize()
                .map_err(|e| format!("{left}: {e}"))?;
            let right = std::path::Path::new(right)
                .canonicalize()
                .map_err(|e| format!("{right}: {e}"))?;
            Ok(left == right)
        }
        "-nt" | "-ot" => {
            let left_meta = std::fs::metadata(left).ok();
            let right_meta = std::fs::metadata(right).ok();
            Ok(match (left_meta, right_meta, op) {
                (Some(_), None, "-nt") => true,
                (None, Some(_), "-ot") => true,
                (None, None, _) => false,
                (None, Some(_), "-nt") => false,
                (Some(_), None, "-ot") => false,
                (Some(left_meta), Some(right_meta), "-nt") => {
                    let left_time = left_meta.modified().map_err(|e| format!("{left}: {e}"))?;
                    let right_time = right_meta.modified().map_err(|e| format!("{right}: {e}"))?;
                    left_time > right_time
                }
                (Some(left_meta), Some(right_meta), "-ot") => {
                    let left_time = left_meta.modified().map_err(|e| format!("{left}: {e}"))?;
                    let right_time = right_meta.modified().map_err(|e| format!("{right}: {e}"))?;
                    left_time < right_time
                }
                _ => false,
            })
        }
        "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
            let l: i64 = left
                .trim()
                .parse()
                .map_err(|_| format!("integer expression expected: `{left}'"))?;
            let r: i64 = right
                .trim()
                .parse()
                .map_err(|_| format!("integer expression expected: `{right}'"))?;
            let result = match op {
                "-eq" => l == r,
                "-ne" => l != r,
                "-lt" => l < r,
                "-le" => l <= r,
                "-gt" => l > r,
                "-ge" => l >= r,
                _ => unreachable!(),
            };
            Ok(result)
        }
        _ => Err(format!("unknown binary operator: `{op}'")),
    }
}

// ── trap implementation ──────────────────────────────────────────────

/// Known POSIX signal names.
const SIGNAL_NAMES: &[&str] = &[
    "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "BUS", "FPE", "KILL", "USR1", "SEGV", "USR2",
    "PIPE", "ALRM", "TERM", "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN", "TTOU", "URG",
    "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "IO", "PWR", "SYS",
];

/// Special (pseudo-)signals recognized by POSIX shells.
const SPECIAL_TRAPS: &[&str] = &["EXIT", "ERR", "DEBUG", "RETURN"];

fn is_valid_signal(name: &str) -> bool {
    // Accept with or without "SIG" prefix, and special traps.
    let normalized = name.strip_prefix("SIG").unwrap_or(name);
    SIGNAL_NAMES
        .iter()
        .any(|s| s.eq_ignore_ascii_case(normalized))
        || SPECIAL_TRAPS.iter().any(|s| s.eq_ignore_ascii_case(name))
        || name.parse::<u32>().is_ok() // numeric signal
}

fn normalize_signal(name: &str) -> String {
    let stripped = name.strip_prefix("SIG").unwrap_or(name);
    stripped.to_uppercase()
}

fn builtin_trap(argv: &[String], env: &mut Env) -> ExecResult {
    use crate::env::TrapAction;

    // trap (no args): list all traps
    if argv.is_empty() {
        let traps = env.traps();
        let mut lines: Vec<String> = traps
            .iter()
            .map(|(sig, trap)| format!("trap -- '{}' {}\n", trap.action, sig))
            .collect();
        lines.sort_unstable();
        return ExecResult {
            exit_code: 0,
            stdout: lines.concat().into_bytes(),
            stderr: Vec::new(),
        };
    }

    // trap -l: list signal names
    if argv.len() == 1 && argv[0] == "-l" {
        let mut output = String::new();
        for (i, name) in SIGNAL_NAMES.iter().enumerate() {
            if i > 0 {
                output.push(' ');
            }
            output.push_str(&format!("{}) SIG{}", i + 1, name));
        }
        output.push('\n');
        return ExecResult {
            exit_code: 0,
            stdout: output.into_bytes(),
            stderr: Vec::new(),
        };
    }

    // trap -p SIGNAL...: print traps for specific signals
    if argv.first().map(|s| s.as_str()) == Some("-p") {
        let signals = &argv[1..];
        let mut output = String::new();
        for sig in signals {
            let norm = normalize_signal(sig);
            if let Some(trap) = env.get_trap(&norm) {
                output.push_str(&format!("trap -- '{}' {}\n", trap.action, norm));
            }
        }
        return ExecResult {
            exit_code: 0,
            stdout: output.into_bytes(),
            stderr: Vec::new(),
        };
    }

    // trap action SIGNAL [SIGNAL...] or trap - SIGNAL
    if argv.len() < 2 {
        return ExecResult::failure(2, "mash: trap: usage: trap [-lp] [[action] signal ...]\n");
    }

    let action = &argv[0];
    let signals = &argv[1..];

    for sig in signals {
        let norm = normalize_signal(sig);
        if !is_valid_signal(sig) {
            return ExecResult::failure(
                1,
                format!("mash: trap: {sig}: invalid signal specification\n"),
            );
        }

        if action == "-" {
            // Reset to default
            env.clear_trap(&norm);
        } else {
            env.set_trap(
                norm,
                TrapAction {
                    action: action.clone(),
                    inherited: false,
                },
            );
        }
    }

    ExecResult::success()
}

fn builtin_history(argv: &[String], env: &mut Env) -> ExecResult {
    if argv.len() == 1 && argv[0] == "-c" {
        env.clear_history();
        return ExecResult::success();
    }

    if !argv.is_empty() {
        return ExecResult::failure(1, "mash: history: unsupported option\n");
    }

    let mut stdout = String::new();
    for (index, entry) in env.history_entries().iter().enumerate() {
        stdout.push_str(&format!("{:>5}  {}\n", index + 1, entry));
    }

    ExecResult {
        exit_code: 0,
        stdout: stdout.into_bytes(),
        stderr: Vec::new(),
    }
}

fn builtin_jobs(argv: &[String], env: &mut Env) -> ExecResult {
    let long_format = argv.iter().any(|arg| arg == "-l");
    let mut stdout = String::new();

    for job in env.jobs() {
        let status = match &job.status {
            crate::env::JobStatus::Running => "Running",
            crate::env::JobStatus::Stopped => "Stopped",
            crate::env::JobStatus::Done => "Done",
            crate::env::JobStatus::Signaled(_) => "Terminated",
        };
        if long_format {
            stdout.push_str(&format!(
                "[{}] {} {} {}\n",
                job.job_id, job.pid, status, job.command
            ));
        } else {
            stdout.push_str(&format!("[{}] {} {}\n", job.job_id, status, job.command));
        }
    }

    ExecResult {
        exit_code: 0,
        stdout: stdout.into_bytes(),
        stderr: Vec::new(),
    }
}

fn builtin_kill(argv: &[String], env: &mut Env) -> ExecResult {
    if argv.is_empty() {
        return ExecResult::failure(1, "mash: kill: usage: kill [-s signal | -signal] pid ...\n");
    }

    let mut signal_spec = "TERM";
    let mut target_start = 0usize;
    if argv.first().map(|s| s.as_str()) == Some("-s") {
        if argv.len() < 3 {
            return ExecResult::failure(
                1,
                "mash: kill: usage: kill [-s signal | -signal] pid ...\n",
            );
        }
        signal_spec = &argv[1];
        target_start = 2;
    } else if argv[0].starts_with('-') && argv[0].len() > 1 {
        signal_spec = &argv[0][1..];
        target_start = 1;
    }

    let Some((signal_name, signal_number)) = resolve_signal_spec(signal_spec) else {
        return ExecResult::failure(
            1,
            format!(
                "mash: kill: {}: invalid signal specification\n",
                signal_spec
            ),
        );
    };
    if target_start >= argv.len() {
        return ExecResult::failure(1, "mash: kill: usage: kill [-s signal | -signal] pid ...\n");
    }

    let shell_pid = env.get_str("$").parse::<u32>().unwrap_or_default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for target in &argv[target_start..] {
        let pid = if target.starts_with('%') {
            if !env.options().monitor {
                stderr
                    .extend_from_slice(format!("mash: kill: {}: invalid pid\n", target).as_bytes());
                exit_code = 1;
                continue;
            }
            match env.job_pid_from_spec(target) {
                Some(pid) => pid,
                None => {
                    stderr.extend_from_slice(
                        format!("mash: kill: {}: no such job\n", target).as_bytes(),
                    );
                    exit_code = 1;
                    continue;
                }
            }
        } else {
            let Ok(pid) = target.parse::<u32>() else {
                stderr
                    .extend_from_slice(format!("mash: kill: {}: invalid pid\n", target).as_bytes());
                exit_code = 1;
                continue;
            };
            pid
        };

        if pid == shell_pid {
            if signal_number == 0 {
                continue;
            }
            if let Some(trap) = env.get_trap(&signal_name).cloned() {
                let trap_result = execute_trap_action(&trap.action, env);
                stdout.extend_from_slice(&trap_result.stdout);
                stderr.extend_from_slice(&trap_result.stderr);
                if env.options().errexit && trap_result.exit_code != 0 {
                    env.request_exit(trap_result.exit_code);
                    exit_code = trap_result.exit_code;
                }
            } else {
                env.request_exit(128 + signal_number);
            }
            continue;
        }

        if signal_number == 0 {
            if !env.jobs().iter().any(|job| job.pid == pid) {
                stderr.extend_from_slice(
                    format!("mash: kill: {}: no such process\n", pid).as_bytes(),
                );
                exit_code = 1;
            }
            continue;
        }

        if env.signal_job(pid, signal_name.clone(), 128 + signal_number) {
        } else {
            stderr.extend_from_slice(format!("mash: kill: {}: no such process\n", pid).as_bytes());
            exit_code = 1;
        }
    }

    ExecResult {
        exit_code,
        stdout,
        stderr,
    }
}

fn builtin_wait(argv: &[String], env: &mut Env) -> ExecResult {
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    let targets: Vec<u32> = if argv.is_empty() {
        env.jobs().into_iter().map(|job| job.pid).collect()
    } else {
        let mut resolved = Vec::new();
        for spec in argv {
            match env.job_pid_from_spec(spec) {
                Some(pid) => resolved.push(pid),
                None => {
                    stderr.extend_from_slice(
                        format!("mash: wait: {}: no such job\n", spec).as_bytes(),
                    );
                    exit_code = 1;
                }
            }
        }
        resolved
    };

    for pid in targets {
        match env.wait_for_job(pid) {
            Some(code) => {
                exit_code = code;
                let _ = env.remove_job(pid);
            }
            None => {
                stderr.extend_from_slice(format!("mash: wait: {}: no such job\n", pid).as_bytes());
                exit_code = 127;
            }
        }
    }

    ExecResult {
        exit_code,
        stdout: Vec::new(),
        stderr,
    }
}

fn resolve_job_target(argv: &[String], env: &Env, builtin: &str) -> Result<crate::env::JobEntry, ExecResult> {
    if argv.len() > 1 {
        return Err(ExecResult::failure(
            1,
            format!("mash: {builtin}: usage: {builtin} [job]\n"),
        ));
    }

    let jobs = env.jobs();
    if jobs.is_empty() {
        return Err(ExecResult::failure(1, format!("mash: {builtin}: no current job\n")));
    }

    if argv.is_empty() {
        return Ok(jobs.last().cloned().expect("jobs non-empty"));
    }

    let spec = &argv[0];
    let Some(pid) = env.job_pid_from_spec(spec) else {
        return Err(ExecResult::failure(
            1,
            format!("mash: {builtin}: {}: no such job\n", spec),
        ));
    };
    jobs.into_iter().find(|job| job.pid == pid).ok_or_else(|| {
        ExecResult::failure(1, format!("mash: {builtin}: {}: no such job\n", spec))
    })
}

fn builtin_bg(argv: &[String], env: &mut Env) -> ExecResult {
    let job = match resolve_job_target(argv, env, "bg") {
        Ok(job) => job,
        Err(err) => return err,
    };

    let _ = env.signal_job(job.pid, "CONT".to_string(), 0);
    ExecResult {
        exit_code: 0,
        stdout: format!("[{}] {}\n", job.job_id, job.command).into_bytes(),
        stderr: Vec::new(),
    }
}

fn builtin_fg(argv: &[String], env: &mut Env) -> ExecResult {
    let job = match resolve_job_target(argv, env, "fg") {
        Ok(job) => job,
        Err(err) => return err,
    };

    let _ = env.signal_job(job.pid, "CONT".to_string(), 0);
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    match env.wait_for_job(job.pid) {
        Some(code) => {
            exit_code = code;
            let _ = env.remove_job(job.pid);
        }
        None => {
            stderr.extend_from_slice(format!("mash: fg: {}: no such job\n", job.pid).as_bytes());
            exit_code = 1;
        }
    }

    ExecResult {
        exit_code,
        stdout: format!("{}\n", job.command).into_bytes(),
        stderr,
    }
}

fn resolve_signal_spec(spec: &str) -> Option<(String, i32)> {
    if spec == "0" {
        return Some(("0".to_string(), 0));
    }

    if let Ok(number) = spec.parse::<usize>() {
        let index = number.checked_sub(1)?;
        let name = SIGNAL_NAMES.get(index)?;
        return Some(((*name).to_string(), number as i32));
    }

    let normalized = normalize_signal(spec);
    let number = SIGNAL_NAMES
        .iter()
        .position(|name| *name == normalized)
        .map(|index| index as i32 + 1)?;
    Some((normalized, number))
}

fn execute_trap_action(action: &str, env: &mut Env) -> ExecResult {
    let saved_exit_trap = env.get_trap("EXIT").cloned();
    if saved_exit_trap.is_some() {
        env.clear_trap("EXIT");
    }
    match crate::parser::parse(action) {
        Ok(cmds) => {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_code = 0;
            for cmd in &cmds {
                if env.exit_requested().is_some()
                    || !matches!(env.loop_control(), LoopControl::None)
                {
                    break;
                }
                let result = execute(cmd, action, env);
                exit_code = result.exit_code;
                stdout.extend_from_slice(&result.stdout);
                stderr.extend_from_slice(&result.stderr);
                if env.options().errexit && result.exit_code != 0 {
                    break;
                }
            }
            if let Some(trap) = saved_exit_trap {
                env.set_trap("EXIT".to_string(), trap);
            }
            ExecResult {
                exit_code,
                stdout,
                stderr,
            }
        }
        Err(e) => {
            if let Some(trap) = saved_exit_trap {
                env.set_trap("EXIT".to_string(), trap);
            }
            ExecResult::failure(1, format!("mash: trap: {e}\n"))
        }
    }
}

// ── type implementation ──────────────────────────────────────────────

/// Shell reserved words / keywords.
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "function", "select", "time", "coproc", "{", "}", "[[", "]]", "!",
];

/// Names recognized as builtins by this shell.
const BUILTIN_NAMES: &[&str] = &[
    "break", "continue", "return", "exit", "true", ":", "false", "echo", "local", "eval", "set",
    "shift", "source", ".", "export", "unset", "readonly", "cd", "pwd", "test", "[", "trap",
    "history", "jobs", "kill", "wait", "bg", "fg", "type", "hash", "command", "read", "printf",
    "alias", "unalias", "getopts", "umask", "times", "exec", "shopt",
];

fn is_special_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "." | ":"
            | "break"
            | "continue"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "times"
            | "trap"
            | "unset"
    )
}

fn builtin_shopt(argv: &[String], env: &mut Env) -> ExecResult {
    let mut set_opt: Option<(String, bool)> = None;
    let mut opt_names: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "-s" => {
                // Set option
                i += 1;
                if i < argv.len() {
                    set_opt = Some((argv[i].clone(), true));
                    opt_names.push(argv[i].clone());
                }
            }
            "-u" => {
                // Unset option
                i += 1;
                if i < argv.len() {
                    set_opt = Some((argv[i].clone(), false));
                    opt_names.push(argv[i].clone());
                }
            }
            "-q" => {
                // Quiet mode - suppress output, just return exit code
                // For now, we just ignore this
            }
            _ => {
                opt_names.push(arg.clone());
            }
        }
        i += 1;
    }

    // Handle set/unset operations
    if let Some((name, value)) = set_opt {
        match name.as_str() {
            "nonlexicalctrl" => {
                env.set_option_nonlexicalctrl(value);
                return ExecResult::success();
            }
            _ => {
                return ExecResult::failure(
                    1,
                    format!("mash: shopt: {name}: invalid shell option\n"),
                );
            }
        }
    }

    // Show specific options or all
    let mut stdout = String::new();
    let show_names: Vec<String> = if opt_names.is_empty() {
        vec!["nonlexicalctrl".to_string()]
    } else {
        opt_names
    };

    for name in &show_names {
        let is_set = match name.as_str() {
            "nonlexicalctrl" => env.options().nonlexicalctrl,
            _ => false,
        };
        stdout.push_str(&format!(
            "{}\t{}\n",
            if is_set { "on" } else { "off" },
            name.as_str()
        ));
    }

    ExecResult {
        exit_code: 0,
        stdout: stdout.into_bytes(),
        stderr: Vec::new(),
    }
}

fn builtin_type(argv: &[String], env: &Env) -> ExecResult {
    let mut type_only = false;
    let mut names: &[String] = argv;

    if let Some(first) = argv.first() {
        if first == "-t" {
            type_only = true;
            names = &argv[1..];
        }
    }

    if names.is_empty() {
        return ExecResult::failure(1, "mash: type: usage: type [-t] name [name ...]\n");
    }

    let mut stdout = String::new();
    let mut exit_code = 0;

    for name in names {
        let name_str = name.as_str();

        // Check order: alias → keyword → function → builtin → external
        if let Some(alias_val) = env.get_alias(name_str) {
            if type_only {
                stdout.push_str("alias\n");
            } else {
                stdout.push_str(&format!("{} is aliased to `{}'\n", name_str, alias_val));
            }
        } else if SHELL_KEYWORDS.contains(&name_str) {
            if type_only {
                stdout.push_str("keyword\n");
            } else {
                stdout.push_str(&format!("{} is a shell keyword\n", name_str));
            }
        } else if env.get_function(name_str).is_some() {
            if type_only {
                stdout.push_str("function\n");
            } else {
                stdout.push_str(&format!("{} is a function\n", name_str));
            }
        } else if BUILTIN_NAMES.contains(&name_str) {
            if type_only {
                stdout.push_str("builtin\n");
            } else {
                stdout.push_str(&format!("{} is a shell builtin\n", name_str));
            }
        } else if let Some(path) = find_in_path(name_str, env) {
            let path_str = path.to_string_lossy();
            if type_only {
                stdout.push_str("file\n");
            } else {
                stdout.push_str(&format!("{} is {}\n", name_str, path_str));
            }
        } else {
            if !type_only {
                stdout.push_str(&format!("mash: type: {}: not found\n", name_str));
            }
            exit_code = 1;
        }
    }

    ExecResult {
        exit_code,
        stdout: stdout.into_bytes(),
        stderr: Vec::new(),
    }
}

// ── hash implementation ──────────────────────────────────────────────

fn builtin_hash(argv: &[String], env: &mut Env) -> ExecResult {
    // hash -r: clear all cached entries
    if argv.len() == 1 && argv[0] == "-r" {
        env.hash_clear();
        return ExecResult::success();
    }

    // hash -d name: remove specific entry
    if argv.len() == 2 && argv[0] == "-d" {
        env.hash_remove(&argv[1]);
        return ExecResult::success();
    }

    // hash (no args): list cached PATH lookups
    if argv.is_empty() {
        let table = env.hash_table();
        if table.is_empty() {
            return ExecResult {
                exit_code: 0,
                stdout: b"hash: hash table empty\n".to_vec(),
                stderr: Vec::new(),
            };
        }
        let mut lines: Vec<String> = table
            .iter()
            .map(|(name, path)| format!("{}\t{}\n", name, path))
            .collect();
        lines.sort_unstable();
        return ExecResult {
            exit_code: 0,
            stdout: lines.concat().into_bytes(),
            stderr: Vec::new(),
        };
    }

    // hash name [name...]: force cache entries
    let mut exit_code = 0;
    let mut errors = Vec::new();
    for name in argv {
        if let Some(path) = find_in_path(name, env) {
            let path_str = path.to_string_lossy().to_string();
            env.hash_insert(name.clone(), path_str);
        } else {
            errors.extend_from_slice(format!("mash: hash: {}: not found\n", name).as_bytes());
            exit_code = 1;
        }
    }

    ExecResult {
        exit_code,
        stdout: Vec::new(),
        stderr: errors,
    }
}

// ── command implementation ───────────────────────────────────────────

fn builtin_command(
    argv: &[String],
    env: &mut Env,
    stdin_file: Option<std::fs::File>,
) -> ExecResult {
    if argv.is_empty() {
        return ExecResult::success();
    }

    // command -v name: print path or type identifier
    if argv[0] == "-v" {
        if argv.len() < 2 {
            return ExecResult::failure(1, "");
        }
        let mut stdout = String::new();
        let mut exit_code = 0;
        for name in &argv[1..] {
            let name_str = name.as_str();
            if BUILTIN_NAMES.contains(&name_str) {
                stdout.push_str(&format!("{}\n", name_str));
            } else if SHELL_KEYWORDS.contains(&name_str) {
                stdout.push_str(&format!("{}\n", name_str));
            } else if env.get_function(name_str).is_some() {
                stdout.push_str(&format!("{}\n", name_str));
            } else if let Some(path) = find_in_path(name_str, env) {
                stdout.push_str(&format!("{}\n", path.to_string_lossy()));
            } else {
                exit_code = 1;
            }
        }
        return ExecResult {
            exit_code,
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        };
    }

    // command -V name: verbose, like `type`
    if argv[0] == "-V" {
        if argv.len() < 2 {
            return ExecResult::failure(1, "");
        }
        return builtin_type(&argv[1..], env);
    }

    // command name args...: execute bypassing functions and aliases.
    // Check builtins first, then PATH.
    let cmd_name = &argv[0];
    let cmd_argv = &argv[1..];

    // Try builtins (but not `command` itself to avoid infinite recursion).
    if cmd_name != "command" {
        let prior_exit_request = env.exit_requested();
        if let Some(result) = try_execute_builtin(cmd_name, cmd_argv, env, stdin_file) {
            if is_special_builtin_name(cmd_name) {
                env.set_exit_requested(prior_exit_request);
            }
            return result;
        }
    }

    // Try external command via PATH.
    if let Some(path) = find_in_path(cmd_name, env) {
        let mut config = malt_platform::process::SpawnConfig::new(&path);
        config.args = cmd_argv.iter().map(|a| a.into()).collect();
        configure_command_spawn_identity(&mut config, cmd_name, &path);
        config.stdout = malt_platform::process::Io::Pipe;
        config.stderr = malt_platform::process::Io::Pipe;
        config.env_clear = true;
        for (k, v) in env.exported_vars() {
            config.env.push((k.into(), v.into()));
        }
        match malt_platform::process::spawn(config) {
            Ok(mut child) => {
                let mut stdout_bytes = Vec::new();
                let mut stderr_bytes = Vec::new();
                if let Some(mut out) = child.take_stdout() {
                    let _ = out.read_to_end(&mut stdout_bytes);
                }
                if let Some(mut err) = child.take_stderr() {
                    let _ = err.read_to_end(&mut stderr_bytes);
                }
                let exit_code = match wait_for_child_exit_code(&mut child, env) {
                    Ok(code) => code,
                    Err(_) => 1,
                };
                ExecResult {
                    exit_code,
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                }
            }
            Err(e) => {
                let code = match &e {
                    malt_platform::process::SpawnError::NotFound { .. } => 127,
                    malt_platform::process::SpawnError::PermissionDenied { .. } => 126,
                    _ => 1,
                };
                ExecResult::failure(code, format!("mash: {cmd_name}: {e}\n"))
            }
        }
    } else {
        ExecResult::failure(127, format!("mash: {cmd_name}: command not found\n"))
    }
}

// ── read implementation ──────────────────────────────────────────────

fn builtin_read(argv: &[String], env: &mut Env, stdin_file: Option<std::fs::File>) -> ExecResult {
    use std::io::BufRead;

    // Parse options.
    let mut raw_mode = false;
    let mut var_names: Vec<&str> = Vec::new();

    for arg in argv {
        if arg == "-r" {
            raw_mode = true;
        } else {
            var_names.push(arg);
        }
    }

    // Default variable is REPLY.
    if var_names.is_empty() {
        var_names.push("REPLY");
    }

    // Read one line from stdin_file or real stdin.
    let line = if let Some(file) = stdin_file {
        let mut reader = std::io::BufReader::new(file);
        let mut buf = String::new();
        match reader.read_line(&mut buf) {
            Ok(0) => return ExecResult::with_code(1), // EOF
            Ok(_) => buf,
            Err(_) => return ExecResult::with_code(1),
        }
    } else {
        let stdin = std::io::stdin();
        let mut buf = String::new();
        match stdin.lock().read_line(&mut buf) {
            Ok(0) => return ExecResult::with_code(1), // EOF
            Ok(_) => buf,
            Err(_) => return ExecResult::with_code(1),
        }
    };

    // Strip trailing newline.
    let mut line = line
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string();

    // Process backslash continuation if not in raw mode.
    if !raw_mode {
        line = line
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\\\", "\\");
    }

    // Split on IFS (default: space/tab/newline).
    let ifs = env.get_str("IFS");
    let ifs = if ifs.is_empty() { " \t\n" } else { ifs };

    let fields: Vec<&str> = if var_names.len() == 1 {
        vec![line.trim_matches(|c: char| ifs.contains(c))]
    } else {
        split_on_ifs(&line, ifs, var_names.len())
    };

    // Assign to variables.
    for (i, name) in var_names.iter().enumerate() {
        let value = if i < fields.len() {
            fields[i].to_string()
        } else {
            String::new()
        };
        let _ = env.set(name, Variable::string(value));
    }

    ExecResult::success()
}

/// Split a string on IFS characters into at most `max_fields` fields.
/// The last field gets the remainder of the line.
fn split_on_ifs<'a>(input: &'a str, ifs: &str, max_fields: usize) -> Vec<&'a str> {
    let mut fields = Vec::new();
    let trimmed = input.trim_matches(|c: char| ifs.contains(c));
    if trimmed.is_empty() {
        return fields;
    }

    let mut start = 0;
    let mut in_delim = false;

    for (i, ch) in trimmed.char_indices() {
        if ifs.contains(ch) {
            if !in_delim {
                if fields.len() + 1 >= max_fields {
                    // Last field gets remainder.
                    break;
                }
                fields.push(&trimmed[start..i]);
                in_delim = true;
            }
        } else {
            if in_delim {
                start = i;
                in_delim = false;
            }
        }
    }

    // Push the remainder as the last field.
    if start < trimmed.len() {
        fields.push(&trimmed[start..]);
    }

    fields
}

// ── printf implementation ───────────────────────────────────────────

fn builtin_printf(argv: &[String]) -> ExecResult {
    if argv.is_empty() {
        return ExecResult::failure(1, "mash: printf: usage: printf format [arguments]\n");
    }

    let format = &argv[0];
    let args = &argv[1..];
    let mut output = String::new();

    // POSIX: reuse format string if more args than specifiers.
    let mut arg_idx = 0;
    let mut did_consume = true;

    while did_consume {
        did_consume = false;
        let mut chars = format.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                match chars.peek() {
                    Some('%') => {
                        chars.next();
                        output.push('%');
                    }
                    Some('s') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        output.push_str(val);
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('d') | Some('i') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let num: i64 = val.parse().unwrap_or(0);
                        output.push_str(&num.to_string());
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('o') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let num: i64 = val.parse().unwrap_or(0);
                        output.push_str(&format!("{:o}", num));
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('x') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let num: i64 = val.parse().unwrap_or(0);
                        output.push_str(&format!("{:x}", num));
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('X') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let num: i64 = val.parse().unwrap_or(0);
                        output.push_str(&format!("{:X}", num));
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('c') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        if let Some(c) = val.chars().next() {
                            output.push(c);
                        }
                        arg_idx += 1;
                        did_consume = true;
                    }
                    Some('b') => {
                        chars.next();
                        let val = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        output.push_str(&interpret_backslash_escapes(val));
                        arg_idx += 1;
                        did_consume = true;
                    }
                    _ => {
                        // Unknown specifier — output literally.
                        output.push('%');
                    }
                }
            } else if ch == '\\' {
                // Interpret backslash escapes in the format string.
                match chars.peek() {
                    Some('n') => {
                        chars.next();
                        output.push('\n');
                    }
                    Some('t') => {
                        chars.next();
                        output.push('\t');
                    }
                    Some('r') => {
                        chars.next();
                        output.push('\r');
                    }
                    Some('\\') => {
                        chars.next();
                        output.push('\\');
                    }
                    Some('0') => {
                        chars.next();
                        // Read up to 3 octal digits.
                        let mut octal = String::new();
                        for _ in 0..3 {
                            if let Some(&c) = chars.peek() {
                                if c.is_ascii_digit() && c != '8' && c != '9' {
                                    octal.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        let val = u8::from_str_radix(&octal, 8).unwrap_or(0);
                        output.push(val as char);
                    }
                    Some('x') => {
                        chars.next();
                        // Read up to 2 hex digits.
                        let mut hex = String::new();
                        for _ in 0..2 {
                            if let Some(&c) = chars.peek() {
                                if c.is_ascii_hexdigit() {
                                    hex.push(c);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                        if !hex.is_empty() {
                            let val = u8::from_str_radix(&hex, 16).unwrap_or(0);
                            output.push(val as char);
                        }
                    }
                    _ => {
                        output.push('\\');
                    }
                }
            } else {
                output.push(ch);
            }
        }

        // Only loop if we consumed args AND there are still more args.
        if !did_consume || arg_idx >= args.len() {
            break;
        }
    }

    ExecResult {
        exit_code: 0,
        stdout: output.into_bytes(),
        stderr: Vec::new(),
    }
}

/// Interpret backslash escape sequences in a string (for %b format).
fn interpret_backslash_escapes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('a') => {
                    chars.next();
                    result.push('\x07');
                }
                Some('b') => {
                    chars.next();
                    result.push('\x08');
                }
                Some('f') => {
                    chars.next();
                    result.push('\x0C');
                }
                Some('v') => {
                    chars.next();
                    result.push('\x0B');
                }
                Some('0') => {
                    chars.next();
                    let mut octal = String::new();
                    for _ in 0..3 {
                        if let Some(&c) = chars.peek() {
                            if c.is_ascii_digit() && c != '8' && c != '9' {
                                octal.push(c);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }
                    let val = u8::from_str_radix(&octal, 8).unwrap_or(0);
                    result.push(val as char);
                }
                _ => {
                    result.push('\\');
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

// ── alias implementation ────────────────────────────────────────────

fn builtin_alias(argv: &[String], env: &mut Env) -> ExecResult {
    // No args: list all aliases.
    if argv.is_empty() {
        let aliases = env.aliases();
        let mut lines: Vec<String> = aliases
            .iter()
            .map(|(k, v)| format!("alias {}='{}'\n", k, v))
            .collect();
        lines.sort_unstable();
        return ExecResult {
            exit_code: 0,
            stdout: lines.concat().into_bytes(),
            stderr: Vec::new(),
        };
    }

    let mut exit_code = 0;
    let mut stdout = String::new();
    let mut stderr = Vec::new();

    for arg in argv {
        if let Some((name, value)) = arg.split_once('=') {
            // Set alias.
            env.set_alias(name.to_string(), value.to_string());
        } else {
            // Show specific alias.
            match env.get_alias(arg) {
                Some(val) => {
                    stdout.push_str(&format!("alias {}='{}'\n", arg, val));
                }
                None => {
                    stderr
                        .extend_from_slice(format!("mash: alias: {}: not found\n", arg).as_bytes());
                    exit_code = 1;
                }
            }
        }
    }

    ExecResult {
        exit_code,
        stdout: stdout.into_bytes(),
        stderr,
    }
}

// ── unalias implementation ──────────────────────────────────────────

fn builtin_unalias(argv: &[String], env: &mut Env) -> ExecResult {
    if argv.is_empty() {
        return ExecResult::failure(1, "mash: unalias: usage: unalias [-a] name [name ...]\n");
    }

    // unalias -a: remove all.
    if argv.iter().any(|a| a == "-a") {
        env.clear_aliases();
        return ExecResult::success();
    }

    let mut exit_code = 0;
    let mut stderr = Vec::new();

    for name in argv {
        if !env.unset_alias(name) {
            stderr.extend_from_slice(format!("mash: unalias: {}: not found\n", name).as_bytes());
            exit_code = 1;
        }
    }

    ExecResult {
        exit_code,
        stdout: Vec::new(),
        stderr,
    }
}

// ── getopts implementation ──────────────────────────────────────────

fn builtin_getopts(argv: &[String], env: &mut Env) -> ExecResult {
    // getopts OPTSTRING VAR [ARGS...]
    if argv.len() < 2 {
        return ExecResult::failure(
            1,
            "mash: getopts: usage: getopts optstring name [arg ...]\n",
        );
    }

    let optstring = &argv[0];
    let varname = &argv[1];

    // If additional args provided, use those; otherwise use positional params.
    let args: Vec<String> = if argv.len() > 2 {
        argv[2..].to_vec()
    } else {
        let count: usize = env.get_str("#").parse().unwrap_or(0);
        (1..=count)
            .map(|i| env.get_str(&i.to_string()).to_string())
            .collect()
    };

    let silent = optstring.starts_with(':');
    let opts = if silent {
        &optstring[1..]
    } else {
        optstring.as_str()
    };

    // Get current OPTIND (1-based index into args).
    let optind: usize = env.get_str("OPTIND").parse().unwrap_or(1);
    let idx = optind.saturating_sub(1); // Convert to 0-based.

    if idx >= args.len() {
        let _ = env.set(varname, Variable::string("?"));
        return ExecResult::with_code(1);
    }

    let current_arg = &args[idx];

    // Not an option (doesn't start with -) or is just "-".
    if !current_arg.starts_with('-') || current_arg == "-" {
        let _ = env.set(varname, Variable::string("?"));
        return ExecResult::with_code(1);
    }

    // "--" signals end of options.
    if current_arg == "--" {
        let _ = env.set(varname, Variable::string("?"));
        let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
        return ExecResult::with_code(1);
    }

    // Get the option character (skip leading '-').
    // We handle one option per invocation. For multi-char args like "-abc",
    // we process one char per call using an internal offset.
    let opt_chars: Vec<char> = current_arg[1..].chars().collect();

    // Use a sub-index for multi-char option groups (stored as part of OPTIND).
    // For simplicity, we process the first unprocessed char.
    // POSIX getopts processes one option per call.
    let opt_char = opt_chars[0];

    // Look up in optstring.
    let pos = opts.find(opt_char);

    match pos {
        Some(p) => {
            let needs_arg = opts.get(p + 1..p + 2) == Some(":");

            if needs_arg {
                // Check if argument follows in the same word.
                if opt_chars.len() > 1 {
                    let optarg: String = opt_chars[1..].iter().collect();
                    let _ = env.set("OPTARG", Variable::string(optarg));
                    let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
                } else if idx + 1 < args.len() {
                    // Next arg is the option argument.
                    let _ = env.set("OPTARG", Variable::string(args[idx + 1].clone()));
                    let _ = env.set("OPTIND", Variable::string((optind + 2).to_string()));
                } else {
                    // Missing argument.
                    if silent {
                        let _ = env.set(varname, Variable::string(":"));
                        let _ = env.set("OPTARG", Variable::string(opt_char.to_string()));
                    } else {
                        let _ = env.set(varname, Variable::string("?"));
                        return ExecResult::failure(
                            0,
                            format!("mash: getopts: option requires an argument -- {opt_char}\n"),
                        );
                    }
                    let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
                    return ExecResult::success();
                }
            } else {
                // No argument needed. Advance OPTIND based on whether more chars remain.
                if opt_chars.len() > 1 {
                    // Multi-char group — for simplicity we only handle single-char options
                    // per invocation. Advance OPTIND.
                    let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
                } else {
                    let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
                }
                let _ = env.unset("OPTARG");
            }

            let _ = env.set(varname, Variable::string(opt_char.to_string()));
            ExecResult::success()
        }
        None => {
            // Unknown option.
            if silent {
                let _ = env.set(varname, Variable::string("?"));
                let _ = env.set("OPTARG", Variable::string(opt_char.to_string()));
            } else {
                let _ = env.set(varname, Variable::string("?"));
                let _ = env.unset("OPTARG");
            }
            let _ = env.set("OPTIND", Variable::string((optind + 1).to_string()));
            ExecResult::success()
        }
    }
}

// ── umask implementation (stub) ─────────────────────────────────────

/// Stub umask builtin. Accepts and validates arguments but uses a default
/// value on platforms without direct umask support.
/// TODO: delegate to malt-platform when OS abstraction layer exposes umask.
fn builtin_umask(argv: &[String]) -> ExecResult {
    if argv.is_empty() {
        return ExecResult {
            exit_code: 0,
            stdout: b"0022\n".to_vec(),
            stderr: Vec::new(),
        };
    }

    if argv.len() == 1 && argv[0] == "-S" {
        return ExecResult {
            exit_code: 0,
            stdout: b"u=rwx,g=rx,o=rx\n".to_vec(),
            stderr: Vec::new(),
        };
    }

    // Validate the octal number but don't actually set (stub).
    if let Some(val_str) = argv.last() {
        if val_str != "-S" {
            if u32::from_str_radix(val_str, 8).is_err() {
                return ExecResult::failure(
                    1,
                    format!("mash: umask: {val_str}: invalid octal number\n"),
                );
            }
        }
    }

    ExecResult::success()
}

// ── If statement ──────────────────────────────────────────────────────

fn execute_if(
    condition: &Spanned<Command>,
    then_body: &[Spanned<Command>],
    elif_clauses: &[(Spanned<Command>, Vec<Spanned<Command>>)],
    else_body: Option<&[Spanned<Command>]>,
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();

    // Suppress errexit for condition evaluation (POSIX).
    let prev_suppress_errexit = env.suppress_errexit();
    env.set_suppress_errexit(true);
    let cond_result = execute(condition, source, env);
    env.set_suppress_errexit(prev_suppress_errexit);
    all_stdout.extend_from_slice(&cond_result.stdout);
    all_stderr.extend_from_slice(&cond_result.stderr);

    if cond_result.exit_code == 0 {
        let mut body_result = execute_list(then_body, source, env);
        body_result.stdout.splice(0..0, all_stdout);
        body_result.stderr.splice(0..0, all_stderr);
        return body_result;
    }

    // Check elif clauses.
    for (elif_cond, elif_body) in elif_clauses {
        env.set_suppress_errexit(true);
        let elif_result = execute(elif_cond, source, env);
        env.set_suppress_errexit(prev_suppress_errexit);
        all_stdout.extend_from_slice(&elif_result.stdout);
        all_stderr.extend_from_slice(&elif_result.stderr);

        if elif_result.exit_code == 0 {
            let mut body_result = execute_list(elif_body, source, env);
            body_result.stdout.splice(0..0, all_stdout);
            body_result.stderr.splice(0..0, all_stderr);
            return body_result;
        }
    }

    // Else body.
    if let Some(body) = else_body {
        let mut body_result = execute_list(body, source, env);
        body_result.stdout.splice(0..0, all_stdout);
        body_result.stderr.splice(0..0, all_stderr);
        return body_result;
    }

    ExecResult {
        exit_code: 0,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

// ── While / Until ─────────────────────────────────────────────────────

fn execute_while_until(
    condition: &Spanned<Command>,
    body: &[Spanned<Command>],
    is_until: bool,
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();
    let mut last_code = 0i32;
    let prev_depth = env.loop_depth();
    env.set_loop_depth(prev_depth + 1);

    loop {
        if env.exit_requested().is_some() {
            break;
        }

        // Suppress errexit for condition.
        let prev_suppress_errexit = env.suppress_errexit();
        env.set_suppress_errexit(true);
        let cond_result = execute(condition, source, env);
        env.set_suppress_errexit(prev_suppress_errexit);

        let should_continue = if is_until {
            cond_result.exit_code != 0
        } else {
            cond_result.exit_code == 0
        };

        if !should_continue {
            break;
        }

        let result = execute_list(body, source, env);
        last_code = result.exit_code;
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        // Handle loop control.
        match env.loop_control().clone() {
            LoopControl::Break(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                    // Continue to next iteration.
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Continue(n - 1));
                    break;
                }
            }
            LoopControl::Return(code) => {
                last_code = code;
                break;
            }
            LoopControl::None => {}
        }
    }

    env.set_loop_depth(prev_depth);
    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

// ── For loop ──────────────────────────────────────────────────────────

fn execute_for(
    var: &Span,
    words: &[Span],
    body: &[Spanned<Command>],
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let var_name = var.text(source);

    // Expand words.
    let mut expanded_words: Vec<String> = Vec::new();
    for word_span in words {
        let word_text = word_span.text(source);
        match expander::expand_word(word_text, env) {
            Ok(fields) => expanded_words.extend(fields),
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        }
    }

    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();
    let mut last_code = 0i32;
    let prev_depth = env.loop_depth();
    env.set_loop_depth(prev_depth + 1);

    for word in &expanded_words {
        if env.exit_requested().is_some() {
            break;
        }

        if let Err(e) = env.set(var_name, Variable::string(word.clone())) {
            all_stderr.extend_from_slice(format!("mash: {e}\n").as_bytes());
            last_code = 1;
            break;
        }

        let result = execute_list(body, source, env);
        last_code = result.exit_code;
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        // Handle loop control.
        match env.loop_control().clone() {
            LoopControl::Break(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Continue(n - 1));
                    break;
                }
            }
            LoopControl::Return(code) => {
                last_code = code;
                break;
            }
            LoopControl::None => {}
        }
    }

    env.set_loop_depth(prev_depth);
    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

// ── For arithmetic ────────────────────────────────────────────────────

fn execute_for_arith(
    init: &Span,
    cond: &Span,
    step: &Span,
    body: &[Spanned<Command>],
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let init_text = init.text(source);
    let cond_text = cond.text(source);
    let step_text = step.text(source);

    // Evaluate init.
    if !init_text.trim().is_empty() {
        if let Err(e) = expander::eval_arithmetic(init_text, env) {
            return ExecResult::failure(1, format!("mash: {e}\n"));
        }
    }

    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();
    let mut last_code = 0i32;
    let prev_depth = env.loop_depth();
    env.set_loop_depth(prev_depth + 1);

    loop {
        if env.exit_requested().is_some() {
            break;
        }

        // Evaluate condition (empty condition = infinite loop).
        if !cond_text.trim().is_empty() {
            match expander::eval_arithmetic(cond_text, env) {
                Ok(val) => {
                    if val == 0 {
                        break;
                    }
                }
                Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
            }
        }

        let result = execute_list(body, source, env);
        last_code = result.exit_code;
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        // Handle loop control.
        match env.loop_control().clone() {
            LoopControl::Break(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                } else if !env.options().nonlexicalctrl && prev_depth == 0 {
                    env.set_loop_control(LoopControl::None);
                } else {
                    env.set_loop_control(LoopControl::Continue(n - 1));
                    break;
                }
            }
            LoopControl::Return(code) => {
                last_code = code;
                break;
            }
            LoopControl::None => {}
        }

        // Evaluate step.
        if !step_text.trim().is_empty() {
            if let Err(e) = expander::eval_arithmetic(step_text, env) {
                return ExecResult::failure(1, format!("mash: {e}\n"));
            }
        }
    }

    env.set_loop_depth(prev_depth);
    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

// ── Case ──────────────────────────────────────────────────────────────

fn execute_case(
    word: &Span,
    items: &[crate::ast::CaseItem],
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let word_text = word.text(source);
    let expanded_word = match expander::expand_word_nosplit(word_text, env) {
        Ok(w) => w,
        Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
    };

    for item in items {
        for pattern_span in &item.patterns {
            let pattern_text = pattern_span.text(source);
            let expanded_pattern = match expander::expand_word_for_case_pattern(pattern_text, env) {
                Ok(p) => p,
                Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
            };

            if expander::shell_pattern_match(&expanded_word, &expanded_pattern) {
                return execute_list(&item.body, source, env);
            }
        }
    }

    ExecResult::success()
}

// ── Arithmetic command ────────────────────────────────────────────────

fn execute_arithmetic(expr: &Span, source: &str, env: &mut Env) -> ExecResult {
    let expr_text = expr.text(source);
    match expander::eval_arithmetic(expr_text, env) {
        Ok(val) => {
            // POSIX: nonzero result → exit 0, zero result → exit 1.
            if val != 0 {
                ExecResult::success()
            } else {
                ExecResult::with_code(1)
            }
        }
        Err(e) => ExecResult::failure(1, format!("mash: {e}\n")),
    }
}

// ── Conditional command ───────────────────────────────────────────────

fn execute_conditional(expr: &Span, source: &str, env: &mut Env) -> ExecResult {
    let expr_text = expr.text(source).trim();

    // Simple [[ expr ]] evaluation — parse the expression tokens.
    let tokens: Vec<&str> = shell_tokenize_conditional(expr_text);

    let result = eval_conditional_tokens(&tokens, env);
    if result {
        ExecResult::success()
    } else {
        ExecResult::with_code(1)
    }
}

/// Tokenize a conditional expression, respecting quotes.
fn shell_tokenize_conditional(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        // Handle quoted strings.
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\t' {
                i += 1;
            }
        }
        tokens.push(&s[start..i]);
    }
    tokens
}

/// Evaluate conditional tokens for [[ ... ]].
fn eval_conditional_tokens(tokens: &[&str], env: &Env) -> bool {
    if tokens.is_empty() {
        return false;
    }

    // Handle negation.
    if tokens[0] == "!" {
        return !eval_conditional_tokens(&tokens[1..], env);
    }

    // Handle logical operators (lowest precedence, left-to-right).
    // Find the rightmost && or || at top level.
    for i in (0..tokens.len()).rev() {
        if tokens[i] == "&&" {
            let left = eval_conditional_tokens(&tokens[..i], env);
            let right = eval_conditional_tokens(&tokens[i + 1..], env);
            return left && right;
        }
        if tokens[i] == "||" {
            let left = eval_conditional_tokens(&tokens[..i], env);
            let right = eval_conditional_tokens(&tokens[i + 1..], env);
            return left || right;
        }
    }

    // Handle parenthesized expressions.
    if tokens.first() == Some(&"(") && tokens.last() == Some(&")") {
        return eval_conditional_tokens(&tokens[1..tokens.len() - 1], env);
    }

    // Unary operators.
    if tokens.len() == 2 {
        let val = unquote_conditional(tokens[1]);
        match tokens[0] {
            "-f" => return std::path::Path::new(&val).is_file(),
            "-d" => return std::path::Path::new(&val).is_dir(),
            "-e" => return std::path::Path::new(&val).exists(),
            "-z" => return val.is_empty(),
            "-n" => return !val.is_empty(),
            "-r" => return std::path::Path::new(&val).exists(), // simplified
            "-w" => return std::path::Path::new(&val).exists(), // simplified
            "-x" => return std::path::Path::new(&val).exists(), // simplified
            "-s" => {
                return std::fs::metadata(&val)
                    .map(|m| m.len() > 0)
                    .unwrap_or(false);
            }
            _ => {}
        }
    }

    // Binary operators.
    if tokens.len() == 3 {
        let left = unquote_conditional(tokens[0]);
        let right = unquote_conditional(tokens[2]);
        match tokens[1] {
            "=" | "==" => return left == right,
            "!=" => return left != right,
            "-eq" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l == r;
            }
            "-ne" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l != r;
            }
            "-lt" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l < r;
            }
            "-le" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l <= r;
            }
            "-gt" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l > r;
            }
            "-ge" => {
                let l: i64 = left.parse().unwrap_or(0);
                let r: i64 = right.parse().unwrap_or(0);
                return l >= r;
            }
            _ => {}
        }
    }

    // Single token: true if non-empty.
    if tokens.len() == 1 {
        let val = unquote_conditional(tokens[0]);
        return !val.is_empty();
    }

    false
}

/// Remove surrounding quotes from a conditional operand.
fn unquote_conditional(s: &str) -> String {
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ── Env assignment ─────────────────────────────────────────────────────

fn execute_env_assign(assigns: &[(Span, Span)], source: &str, env: &mut Env) -> ExecResult {
    let mut exit_code = 0;
    for (key_span, val_span) in assigns {
        let key = key_span.text(source);
        let val_text = val_span.text(source);
        let has_command_substitution = assignment_word_has_command_substitution(val_text);
        let val = match expander::expand_assignment_word_nosplit(val_text, env) {
            Ok(v) => {
                exit_code = if has_command_substitution {
                    env.exit_code()
                } else {
                    0
                };
                v
            }
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        };
        if let Err(e) = env.set(key, Variable::string(val)) {
            return ExecResult::failure(1, format!("mash: {e}\n"));
        }
    }
    ExecResult::with_code(exit_code)
}

fn assignment_word_has_command_substitution(word: &str) -> bool {
    let bytes = word.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
            }
            b'`' => return true,
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => return true,
            _ => i += 1,
        }
    }
    false
}

// ── List (AND-OR chain) ────────────────────────────────────────────────

fn execute_list_node(
    pairs: &[(Spanned<Command>, ListOp)],
    last: &Spanned<Command>,
    source: &str,
    env: &mut Env,
) -> ExecResult {
    let mut result = ExecResult::success();
    // Accumulate stdout/stderr across the whole list.
    let mut all_stdout = Vec::new();
    let mut all_stderr = Vec::new();

    for (cmd, op) in pairs {
        if env.exit_requested().is_some() {
            break;
        }

        result = execute(cmd, source, env);
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        if env.exit_requested().is_some() || !matches!(env.loop_control(), LoopControl::None) {
            if let Some(code) = env.exit_requested() {
                result.exit_code = code;
            }
            result.stdout = all_stdout;
            result.stderr = all_stderr;
            return result;
        }

        match op {
            ListOp::Sequential => {
                // Always continue.
            }
            ListOp::Background => {
                // Scaffold: just continue (true background comes later).
            }
            ListOp::AndIf => {
                if result.exit_code != 0 {
                    env.set_suppress_errexit(true);
                    // Short-circuit: skip the rest until we see OrIf or Sequential.
                    // But since the AST pairs are flat, we need to skip to `last`.
                    // Actually the parser structures AND-OR so each pair is one link.
                    // If the pair fails on AndIf, the next command is `last` which
                    // we should skip. Return current result.
                    result.stdout = all_stdout;
                    result.stderr = all_stderr;
                    if let Some(code) = env.exit_requested() {
                        result.exit_code = code;
                    }
                    return result;
                }
            }
            ListOp::OrIf => {
                if result.exit_code == 0 {
                    env.set_suppress_errexit(true);
                    // Short-circuit on success for OrIf.
                    result.stdout = all_stdout;
                    result.stderr = all_stderr;
                    if let Some(code) = env.exit_requested() {
                        result.exit_code = code;
                    }
                    return result;
                }
            }
        }
    }

    // Execute the last command.
    if env.exit_requested().is_none() {
        result = execute(last, source, env);
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);
    }

    result.stdout = all_stdout;
    result.stderr = all_stderr;
    if let Some(code) = env.exit_requested() {
        result.exit_code = code;
    }
    result
}

/// Convert a redirect back to text form for alias substitution.
fn redirect_to_text(r: &Spanned<Redirect>, source: &str) -> String {
    let redirect = &r.node;
    let fd_prefix = redirect.fd.map(|f| format!("{}", f)).unwrap_or_default();
    let op = match redirect.kind {
        RedirectKind::Input => "<",
        RedirectKind::Output => ">",
        RedirectKind::Append => ">>",
        RedirectKind::Clobber => ">|",
        RedirectKind::InputOutput => "<>",
        RedirectKind::DupInput => "<&",
        RedirectKind::DupOutput => ">&",
        RedirectKind::Both => "&>",
        RedirectKind::HereString => "<<<",
        RedirectKind::HereDoc => "<<",
        RedirectKind::HereDocStrip => "<<-",
        _ => "",
    };
    format!("{}{} {}", fd_prefix, op, redirect.target.text(source))
}

/// Convert a redirect target path to a platform-native path.
/// Handles `/dev/null` on Windows by mapping it to `NUL`.
fn platformize_path(target: &str) -> String {
    if target == "/dev/null" {
        #[cfg(windows)]
        return "NUL".to_string();
        #[cfg(not(windows))]
        return target.to_string();
    }
    target.to_string()
}

// ── Redirect resolution ────────────────────────────────────────────────

/// Resolved I/O targets from redirect processing.
///
/// Each field is `Some` only if a redirect was specified for that fd.
struct ResolvedIo {
    /// Stdin source (from `< file`, `<<< string`, `<< heredoc`).
    stdin: Option<std::fs::File>,
    /// Stdout target (from `> file`, `>> file`).
    stdout: Option<std::fs::File>,
    /// Stderr target (from `2> file`, `2>> file`).
    stderr: Option<std::fs::File>,
    /// Set when `1>&2` was used and stderr has no explicit file.
    stdout_to_stderr: bool,
    /// Set when `2>&1` was used and stdout has no explicit file.
    stderr_to_stdout: bool,
    /// Extra shell-managed file descriptors opened by redirects like `8<file`.
    extra_fds: HashMap<u8, File>,
    /// Extra shell-managed descriptors that alias one of the shell stdio fds.
    extra_fd_aliases: HashMap<u8, u8>,
    /// Extra shell-managed descriptors backed by snapshot paths.
    extra_fd_snapshots: HashMap<u8, PathBuf>,
    /// Extra descriptors explicitly closed by redirects like `8<&-`.
    closed_fds: Vec<u8>,
}

impl ResolvedIo {
    fn new() -> Self {
        Self {
            stdin: None,
            stdout: None,
            stderr: None,
            stdout_to_stderr: false,
            stderr_to_stdout: false,
            extra_fds: HashMap::new(),
            extra_fd_aliases: HashMap::new(),
            extra_fd_snapshots: HashMap::new(),
            closed_fds: Vec::new(),
        }
    }
}

enum SavedFdState {
    Closed,
    Alias(u32),
    Snapshot(PathBuf),
    File(File),
}

fn save_fd_state(env: &Env, fd: u32) -> SavedFdState {
    if let Some(target) = env.fd_alias_target(fd) {
        return SavedFdState::Alias(target);
    }
    if let Some(path) = env.fd_snapshot_path(fd) {
        return SavedFdState::Snapshot(path);
    }
    if env.has_fd(fd) {
        if let Ok(file) = env.open_fd(fd) {
            return SavedFdState::File(file);
        }
    }
    SavedFdState::Closed
}

fn restore_fd_state(env: &Env, fd: u32, state: SavedFdState) {
    match state {
        SavedFdState::Closed => {
            let _ = env.close_fd(fd);
        }
        SavedFdState::Alias(target) => env.register_fd_alias(fd, target),
        SavedFdState::Snapshot(path) => env.register_fd_snapshot_path(fd, path),
        SavedFdState::File(file) => env.register_fd(fd, file),
    }
}

fn nonstdio_affected_fds(io: &ResolvedIo) -> Vec<u32> {
    let mut fds: Vec<u32> = io
        .extra_fds
        .keys()
        .copied()
        .map(u32::from)
        .chain(io.extra_fd_aliases.keys().copied().map(u32::from))
        .chain(io.extra_fd_snapshots.keys().copied().map(u32::from))
        .chain(
            io.closed_fds
                .iter()
                .copied()
                .filter(|fd| *fd > 2)
                .map(u32::from),
        )
        .collect();
    fds.sort_unstable();
    fds.dedup();
    fds
}

fn apply_nonstdio_redirects(env: &Env, io: &mut ResolvedIo) {
    for fd in io.closed_fds.drain(..) {
        if fd > 2 {
            let _ = env.close_fd(fd as u32);
        }
    }
    for (fd, file) in io.extra_fds.drain() {
        env.register_fd(fd as u32, file);
    }
    for (fd, target_fd) in io.extra_fd_aliases.drain() {
        env.register_fd_alias(fd as u32, target_fd as u32);
    }
    for (fd, path) in io.extra_fd_snapshots.drain() {
        env.register_fd_snapshot_path(fd as u32, path);
    }
}

fn assign_resolved_fd(io: &mut ResolvedIo, fd: u8, file: std::fs::File) {
    match fd {
        0 => io.stdin = Some(file),
        1 => {
            io.stdout = Some(file);
            io.stdout_to_stderr = false;
        }
        2 => {
            io.stderr = Some(file);
            io.stderr_to_stdout = false;
        }
        _ => {
            io.extra_fds.insert(fd, file);
            io.extra_fd_aliases.remove(&fd);
            io.extra_fd_snapshots.remove(&fd);
            io.closed_fds.retain(|closed_fd| *closed_fd != fd);
        }
    }
}

fn assign_resolved_fd_alias(io: &mut ResolvedIo, fd: u8, target_fd: u8) {
    match fd {
        0 => {}
        1 => {
            if target_fd == 2 {
                io.stdout_to_stderr = true;
                io.stdout = None;
            }
        }
        2 => {
            if target_fd == 1 {
                io.stderr_to_stdout = true;
                io.stderr = None;
            }
        }
        _ => {
            io.extra_fds.remove(&fd);
            io.extra_fd_aliases.insert(fd, target_fd);
            io.extra_fd_snapshots.remove(&fd);
            io.closed_fds.retain(|closed_fd| *closed_fd != fd);
        }
    }
}

fn assign_resolved_fd_snapshot(io: &mut ResolvedIo, fd: u8, path: PathBuf) -> std::io::Result<()> {
    match fd {
        0 => {
            io.stdin = Some(std::fs::OpenOptions::new().read(true).open(&path)?);
        }
        1 => {
            let mut file = std::fs::OpenOptions::new().write(true).open(&path)?;
            file.seek(SeekFrom::End(0))?;
            io.stdout = Some(file);
            io.stdout_to_stderr = false;
        }
        2 => {
            let mut file = std::fs::OpenOptions::new().write(true).open(&path)?;
            file.seek(SeekFrom::End(0))?;
            io.stderr = Some(file);
            io.stderr_to_stdout = false;
        }
        _ => {
            io.extra_fds.remove(&fd);
            io.extra_fd_aliases.remove(&fd);
            io.extra_fd_snapshots.insert(fd, path);
            io.closed_fds.retain(|closed_fd| *closed_fd != fd);
        }
    }
    Ok(())
}

fn redirect_effective_fd(redirect: &Redirect) -> u8 {
    redirect
        .fd
        .map(|fd| fd as u8)
        .unwrap_or(match redirect.kind {
            RedirectKind::Input
            | RedirectKind::InputOutput
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => 0,
            _ => 1,
        })
}

/// Process a redirect list into opened file handles.
///
/// Redirects are processed left-to-right per POSIX. The last redirect for
/// a given fd wins. Returns `Err(ExecResult)` on I/O failures.
fn resolve_redirects(
    redirects: &[Spanned<Redirect>],
    source: &str,
    env: &mut Env,
) -> Result<ResolvedIo, ExecResult> {
    let mut io = ResolvedIo::new();

    for (redirect_idx, redir_spanned) in redirects.iter().enumerate() {
        let redirect = &redir_spanned.node;

        // Default fd: 0 for input-like redirects, 1 for output-like.
        let fd = redirect_effective_fd(redirect);

        // Expand the target. For heredoc/herestring the target span contains
        // the body text directly.
        let raw_target = redirect.target.text(source);

        // For heredoc kinds, expand the body if not quoted.
        // Use heredoc_body field if present (new style), otherwise fall back to span text.
        let target: String = match redirect.kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
                let body_text = redirect.heredoc_body.as_deref().unwrap_or(raw_target);
                if redirect.quoted {
                    body_text.to_string()
                } else {
                    match expander::expand_heredoc_body(body_text, env) {
                        Ok(s) => s,
                        Err(e) => {
                            if !env.is_interactive() {
                                env.request_exit(1);
                            }
                            let msg = format!("{e}\n");
                            return Err(ExecResult::failure(1, msg));
                        }
                    }
                }
            }
            RedirectKind::HereString => {
                // Expand the here-string word.
                match expander::expand_word_nosplit(raw_target, env) {
                    Ok(s) => s,
                    Err(e) => {
                        let msg = format!("mash: here-string expansion: {e}\n");
                        return Err(ExecResult::failure(1, msg));
                    }
                }
            }
            _ => {
                // Regular redirect target: expand and use first field.
                match expander::expand_word(raw_target, env) {
                    Ok(fields) => fields.into_iter().next().unwrap_or_default(),
                    Err(e) => {
                        let msg = format!("mash: redirect: {e}\n");
                        return Err(ExecResult::failure(1, msg));
                    }
                }
            }
        };
        let target: &str = &target;
        let platform_target = platformize_path(target);
        let target: &str = &platform_target;

        match redirect.kind {
            RedirectKind::Output | RedirectKind::Clobber => {
                // Check noclobber for Output (not Clobber).
                if redirect.kind == RedirectKind::Output && env.options().noclobber {
                    let p = std::path::Path::new(target);
                    let is_regular = p.metadata().map(|m| m.is_file()).unwrap_or(false);
                    if is_regular {
                        let msg = format!("mash: {target}: cannot overwrite existing file\n");
                        return Err(ExecResult::failure(1, msg));
                    }
                }
                let file = open_redirect_file(target, malt_platform::DevOpenMode::Write, env)
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: {e}\n")))?;
                assign_resolved_fd(&mut io, fd, file);
            }
            RedirectKind::Append => {
                let file = open_append_redirect_file(target, env)
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: {e}\n")))?;
                assign_resolved_fd(&mut io, fd, file);
            }
            RedirectKind::Input => {
                let file = open_redirect_file(target, malt_platform::DevOpenMode::Read, env)
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: {e}\n")))?;
                assign_resolved_fd(&mut io, fd, file);
            }
            RedirectKind::HereDoc | RedirectKind::HereDocStrip | RedirectKind::HereString => {
                let (read, mut write) = malt_platform::io::create_pipe()
                    .map_err(|e| ExecResult::failure(1, format!("mash: pipe: {e}\n")))?;
                let data = if redirect.kind == RedirectKind::HereString {
                    format!("{target}\n")
                } else {
                    target.to_string()
                };
                // For small heredocs, wait until the writer has populated the pipe.
                // This avoids a Windows race where the reader can observe EOF before
                // the background writer runs.
                if data.len() <= 8192 {
                    let (tx, rx) = std::sync::mpsc::sync_channel(1);
                    std::thread::spawn(move || {
                        let result = write.write_all(data.as_bytes());
                        let _ = tx.send(result);
                    });
                    match rx.recv() {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            return Err(ExecResult::failure(
                                1,
                                format!("mash: heredoc write failed: {e}\n"),
                            ));
                        }
                        Err(e) => {
                            return Err(ExecResult::failure(
                                1,
                                format!("mash: heredoc writer failed: {e}\n"),
                            ));
                        }
                    }
                } else {
                    std::thread::spawn(move || {
                        let _ = write.write_all(data.as_bytes());
                    });
                }
                assign_resolved_fd(&mut io, fd, read);
            }
            RedirectKind::Both => {
                // &> file — redirect both stdout and stderr.
                let file = std::fs::File::create(target)
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: {e}\n")))?;
                let file2 = file
                    .try_clone()
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: clone: {e}\n")))?;
                io.stdout = Some(file);
                io.stderr = Some(file2);
                io.stdout_to_stderr = false;
                io.stderr_to_stdout = false;
            }
            RedirectKind::InputOutput => {
                // <> file — open for reading and writing.
                let file = open_redirect_file(target, malt_platform::DevOpenMode::ReadWrite, env)
                    .map_err(|e| ExecResult::failure(1, format!("mash: {target}: {e}\n")))?;
                assign_resolved_fd(&mut io, fd, file);
            }
            RedirectKind::DupInput | RedirectKind::DupOutput => {
                if target == "-" {
                    // Close fd.
                    match fd {
                        0 => io.stdin = None,
                        1 => io.stdout = None,
                        2 => io.stderr = None,
                        _ => {
                            io.extra_fds.remove(&fd);
                            io.extra_fd_aliases.remove(&fd);
                            io.extra_fd_snapshots.remove(&fd);
                            io.closed_fds.push(fd);
                        }
                    }
                } else {
                    let src: u8 = target.parse::<u8>().map_err(|_| {
                        ExecResult::failure(1, format!("mash: {target}: bad file descriptor\n"))
                    })?;
                    let prefer_live_stdio_alias = (fd == 2 && src == 1 && io.stdout.is_none())
                        || (fd == 1 && src == 2 && io.stderr.is_none());
                    let alias_target = if src > 2 {
                        io.extra_fd_aliases
                            .get(&src)
                            .copied()
                            .or_else(|| env.fd_alias_target(src as u32).map(|fd| fd as u8))
                    } else {
                        None
                    };
                    let snapshot_path = if src > 2 {
                        io.extra_fd_snapshots
                            .get(&src)
                            .cloned()
                            .or_else(|| env.fd_snapshot_path(src as u32))
                    } else {
                        env.fd_snapshot_path(src as u32)
                    };
                    if src <= 2 {
                        let src_reassigned_later = redirects[redirect_idx + 1..]
                            .iter()
                            .any(|later| redirect_effective_fd(&later.node) == src);
                        let freeze_stdio_snapshot = (fd == 1 && src == 2 && io.stderr.is_none())
                            || (fd == 2 && src == 1 && io.stdout.is_none());
                        if freeze_stdio_snapshot && !src_reassigned_later {
                            assign_resolved_fd_alias(&mut io, fd, src);
                            continue;
                        }
                        if freeze_stdio_snapshot {
                            if let Some(path) = snapshot_path.clone() {
                                assign_resolved_fd_snapshot(&mut io, fd, path).map_err(|e| {
                                    ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                                })?;
                                continue;
                            }
                        }
                    }
                    let cloned_file = match src {
                        1 if fd == 2 && prefer_live_stdio_alias => None,
                        2 if fd == 1 && prefer_live_stdio_alias => None,
                        0 => io
                            .stdin
                            .as_ref()
                            .and_then(|f| f.try_clone().ok())
                            .or_else(|| env.open_fd_read(0).ok()),
                        1 => io
                            .stdout
                            .as_ref()
                            .and_then(|f| f.try_clone().ok())
                            .or_else(|| env.open_fd_write(1).ok()),
                        2 => io
                            .stderr
                            .as_ref()
                            .and_then(|f| f.try_clone().ok())
                            .or_else(|| env.open_fd_write(2).ok()),
                        _ => io
                            .extra_fds
                            .get(&src)
                            .and_then(|f| f.try_clone().ok())
                            .or_else(|| {
                                if redirect.kind == RedirectKind::DupInput {
                                    env.open_fd_read(src as u32).ok()
                                } else {
                                    env.open_fd_write(src as u32).ok()
                                }
                            }),
                    };
                    let allows_merge =
                        (fd == 1 && src == 2) || (fd == 2 && src == 1) || (fd > 2 && src == fd);
                    let allows_stdio_alias = src <= 2;
                    if cloned_file.is_none()
                        && alias_target.is_none()
                        && !allows_merge
                        && !allows_stdio_alias
                    {
                        return Err(ExecResult::failure(
                            1,
                            format!("mash: {src}: bad file descriptor\n"),
                        ));
                    }
                    if fd > 2 {
                        if let Some(path) = snapshot_path {
                            assign_resolved_fd_snapshot(&mut io, fd, path).map_err(|e| {
                                ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                            })?;
                            continue;
                        }
                    }
                    if let Some(file) = cloned_file {
                        match fd {
                            0 => io.stdin = Some(file),
                            1 => {
                                io.stdout = Some(file);
                                io.stdout_to_stderr = false;
                            }
                            2 => {
                                io.stderr = Some(file);
                                io.stderr_to_stdout = false;
                            }
                            _ => {
                                io.extra_fds.insert(fd, file);
                                io.extra_fd_aliases.remove(&fd);
                                io.extra_fd_snapshots.remove(&fd);
                                io.closed_fds.retain(|closed_fd| *closed_fd != fd);
                            }
                        }
                    } else if let Some(target_fd) = alias_target {
                        assign_resolved_fd_alias(&mut io, fd, target_fd);
                    } else {
                        assign_resolved_fd_alias(&mut io, fd, src);
                    }
                }
            }
        }
    }

    Ok(io)
}

fn open_redirect_file(
    target: &str,
    mode: malt_platform::DevOpenMode,
    env: &Env,
) -> std::io::Result<std::fs::File> {
    if let Some(fd_text) = target.strip_prefix("/dev/fd/") {
        let fd = fd_text.parse::<u32>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid /dev/fd path: {target}"),
            )
        })?;
        return match mode {
            malt_platform::DevOpenMode::Read => env.open_fd_read(fd),
            malt_platform::DevOpenMode::Write => env.open_fd_write(fd),
            malt_platform::DevOpenMode::ReadWrite => env.open_fd(fd),
        };
    }

    let path = std::path::Path::new(target);
    if let Some(result) = malt_platform::try_open_virtual_dev(path, mode) {
        return result;
    }

    match mode {
        malt_platform::DevOpenMode::Read => std::fs::File::open(path),
        malt_platform::DevOpenMode::Write => std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path),
        malt_platform::DevOpenMode::ReadWrite => std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path),
    }
}

fn open_append_redirect_file(target: &str, env: &Env) -> std::io::Result<std::fs::File> {
    if let Some(fd_text) = target.strip_prefix("/dev/fd/") {
        let fd = fd_text.parse::<u32>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid /dev/fd path: {target}"),
            )
        })?;
        return env.open_fd_write(fd);
    }

    let path = std::path::Path::new(target);
    if let Some(result) =
        malt_platform::try_open_virtual_dev(path, malt_platform::DevOpenMode::Write)
    {
        return result;
    }

    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(path)
}

fn apply_exec_redirects(env: &Env, io: &mut ResolvedIo) {
    if let Some(file) = io.stdin.take() {
        env.register_fd(0, file);
    }
    if let Some(file) = io.stdout.take() {
        env.register_fd(1, file);
    } else if io.stdout_to_stderr {
        if let Ok(file) = env.open_fd_write(2) {
            env.register_fd(1, file);
        } else {
            env.register_fd_alias(1, 2);
        }
    }
    if let Some(file) = io.stderr.take() {
        env.register_fd(2, file);
    } else if io.stderr_to_stdout {
        if let Ok(file) = env.open_fd_write(1) {
            env.register_fd(2, file);
        } else {
            env.register_fd_alias(2, 1);
        }
    }

    for fd in io.closed_fds.drain(..) {
        let _ = env.close_fd(fd as u32);
    }

    for (fd, file) in io.extra_fds.drain() {
        env.register_fd(fd as u32, file);
    }

    for (fd, target_fd) in io.extra_fd_aliases.drain() {
        env.register_fd_alias(fd as u32, target_fd as u32);
    }

    for (fd, path) in io.extra_fd_snapshots.drain() {
        env.register_fd_snapshot_path(fd as u32, path);
    }
}

fn add_runtime_spawn_env(config: &mut malt_platform::process::SpawnConfig, env: &Env) {
    if let Some(fd_aliases) = env.fd_alias_env_spec() {
        config
            .env
            .push(("MASH_FD_ALIASES".into(), fd_aliases.into()));
    }
    if let Some(fd_snapshots) = env.fd_snapshot_env_spec() {
        config
            .env
            .push(("MASH_FD_SNAPSHOTS".into(), fd_snapshots.into()));
    }
    config
        .env
        .push(("MASH_PPID".into(), env.get_str("$").to_string().into()));
    #[cfg(unix)]
    {
        for fd in env.nonstdio_fds() {
            let Ok(target_fd) = i32::try_from(fd) else {
                continue;
            };
            if let Ok(file) = env.open_fd(fd) {
                config.extra_fds.push((target_fd, file));
            }
        }
    }
}

fn explicit_internal_command_name(cmd_name: &str) -> Option<String> {
    let normalized = cmd_name.replace('\\', "/");
    for prefix in ["/bin/", "/usr/bin/", "/usr/local/bin/"] {
        if let Some(name) = normalized.strip_prefix(prefix) {
            return (!name.is_empty() && !name.contains('/')).then(|| name.to_string());
        }
    }
    None
}

fn should_execute_shell_script_with_mash(path: &Path) -> bool {
    #[cfg(windows)]
    {
        if !path.is_file() || !malt_platform::fs::is_readable(path) {
            return false;
        }
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => matches!(ext.to_ascii_lowercase().as_str(), "sh" | "bash" | "msh"),
            None => false,
        }
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn execute_shell_script_with_io(
    cmd_name: &str,
    script_path: &Path,
    argv: &[String],
    child_env: &[(String, String)],
    resolved_io: ResolvedIo,
    env: &mut Env,
    stdin_file: Option<std::fs::File>,
    stdout_file: Option<std::fs::File>,
) -> ExecResult {
    let mash_self = env.get_str("MASH_SELF_EXE");
    if mash_self.is_empty() {
        return ExecResult::failure(
            126,
            format!("mash: {cmd_name}: shell script execution unavailable\n"),
        );
    }

    let mut config = malt_platform::process::SpawnConfig::new(mash_self);
    config.args.push(script_path.as_os_str().into());
    config.args.extend(argv.iter().map(|arg| arg.into()));

    config.stdin = match resolved_io.stdin {
        Some(f) => malt_platform::process::Io::File(f),
        None => match stdin_file {
            Some(f) => malt_platform::process::Io::File(f),
            None => match env.open_fd_read(0) {
                Ok(f) => malt_platform::process::Io::File(f),
                Err(_) => malt_platform::process::Io::Inherit,
            },
        },
    };

    config.stdout = match resolved_io.stdout {
        Some(f) => malt_platform::process::Io::File(f),
        None => match stdout_file {
            Some(f) => malt_platform::process::Io::File(f),
            None if env.fd_snapshot_path(1).is_none() && env.has_fd(1) => {
                match env.open_fd_write(1) {
                    Ok(f) => malt_platform::process::Io::File(f),
                    Err(_) => malt_platform::process::Io::Pipe,
                }
            }
            None => malt_platform::process::Io::Pipe,
        },
    };

    config.stderr = match resolved_io.stderr {
        Some(f) => malt_platform::process::Io::File(f),
        None if env.fd_snapshot_path(2).is_none() && env.has_fd(2) => match env.open_fd_write(2) {
            Ok(f) => malt_platform::process::Io::File(f),
            Err(_) => malt_platform::process::Io::Pipe,
        },
        None => malt_platform::process::Io::Pipe,
    };

    let exported = env.exported_vars();
    for (k, v) in &exported {
        config.env.push((k.into(), v.into()));
    }
    for (k, v) in child_env {
        config.env.push((k.into(), v.into()));
    }
    config.env_clear = true;
    add_runtime_spawn_env(&mut config, env);

    let mut child = match malt_platform::process::spawn(config) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: {e}\n");
            let code = match &e {
                malt_platform::process::SpawnError::NotFound { .. } => 127,
                malt_platform::process::SpawnError::PermissionDenied { .. } => 126,
                _ => 1,
            };
            return ExecResult::failure(code, msg);
        }
    };
    env.report_bg_pid(child.pid());

    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        if let Err(e) = out.read_to_end(&mut stdout_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stdout read failed: {e}\n").as_bytes(),
            );
        }
    }
    if let Some(mut err) = child.take_stderr() {
        if let Err(e) = err.read_to_end(&mut stderr_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stderr read failed: {e}\n").as_bytes(),
            );
        }
    }

    let exit_code = match wait_for_child_exit_code(&mut child, env) {
        Ok(code) => code,
        Err(e) => {
            stderr_bytes
                .extend_from_slice(format!("mash: {cmd_name}: wait failed: {e}\n").as_bytes());
            1
        }
    };

    ExecResult {
        exit_code,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

fn spawn_program_for_command(_cmd_name: &str, resolved_program: &Path) -> PathBuf {
    resolved_program.to_path_buf()
}

fn configure_command_spawn_identity(
    config: &mut malt_platform::process::SpawnConfig,
    cmd_name: &str,
    resolved_program: &Path,
) {
    let spawn_program = spawn_program_for_command(cmd_name, resolved_program);
    config.program = spawn_program;
    config.argv0 = Some(cmd_name.to_string().into());
    #[cfg(windows)]
    config
        .env
        .push(("MASH_ARGV0".into(), cmd_name.to_string().into()));
}

fn execute_expanded_command(
    cmd_name: &str,
    argv: &[String],
    child_env: &[(String, String)],
    mut resolved_io: ResolvedIo,
    env: &mut Env,
) -> ExecResult {
    if let Some(mut result) = try_execute_builtin(cmd_name, argv, env, resolved_io.stdin.take()) {
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }

    let tools_registry = malt_tools::Registry::new();
    if cmd_name == "sleep" && env.current_job_id().is_some() {
        let mut result = execute_interruptible_sleep(argv, env);
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }
    if tools_registry.contains(cmd_name) {
        let stdin_bytes: Vec<u8> = if let Some(mut file) = resolved_io.stdin.take() {
            let mut buf = Vec::new();
            let _ = std::io::Read::read_to_end(&mut file, &mut buf);
            buf
        } else {
            Vec::new()
        };

        let tool_fn = tools_registry.get(cmd_name).unwrap();
        let tool_result = tool_fn(argv, &stdin_bytes);
        let mut result = ExecResult {
            exit_code: tool_result.exit_code,
            stdout: tool_result.stdout,
            stderr: tool_result.stderr,
        };
        apply_output_redirects(&mut result, resolved_io);
        return result;
    }

    if let Some(func_def) = env.get_function(cmd_name).cloned() {
        if env.call_depth() >= 50 {
            let msg = format!("mash: {cmd_name}: maximum function nesting level exceeded\n");
            return ExecResult::failure(1, msg);
        }

        env.push_scope();
        let saved = env.save_positional();
        env.replace_positional_args(argv);
        env.push_call(CallFrame {
            name: cmd_name.to_string(),
            file: String::new(),
            line: 0,
        });

        let saved_loop_depth = if env.options().nonlexicalctrl {
            env.loop_depth()
        } else {
            let prev = env.loop_depth();
            env.set_loop_depth(0);
            prev
        };

        for (k, v) in child_env {
            let _ = env.set(k, Variable::string(v.clone()));
        }

        let stored_source = func_def.source.clone();
        let func_body = func_def.body.clone();
        let mut result = execute(&func_body, &stored_source, env);

        match env.loop_control().clone() {
            LoopControl::Return(code) => {
                result.exit_code = code;
                env.set_loop_control(LoopControl::None);
            }
            LoopControl::Break(_) | LoopControl::Continue(_) => {
                if !env.options().nonlexicalctrl {
                    env.set_loop_control(LoopControl::None);
                }
            }
            LoopControl::None => {}
        }

        env.set_loop_depth(saved_loop_depth);
        env.pop_call();
        env.restore_positional(saved);
        let _ = env.pop_scope();

        apply_output_redirects(&mut result, resolved_io);
        return result;
    }

    let program = match find_in_path(cmd_name, env) {
        Some(p) => p,
        None => {
            let msg = format!("mash: {cmd_name}: command not found\n");
            return ExecResult::failure(127, msg);
        }
    };

    let merge_stderr_to_stdout = resolved_io.stderr_to_stdout && resolved_io.stderr.is_none();
    let merge_stdout_to_stderr = resolved_io.stdout_to_stderr && resolved_io.stdout.is_none();

    let mut config = malt_platform::process::SpawnConfig::new(&program);
    config.args = argv.iter().map(|a| a.into()).collect();
    configure_command_spawn_identity(&mut config, cmd_name, &program);
    config.stdin = match resolved_io.stdin {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Inherit,
    };
    config.stdout = match resolved_io.stdout {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Pipe,
    };
    config.stderr = match resolved_io.stderr {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Pipe,
    };

    let exported = env.exported_vars();
    for (k, v) in &exported {
        config.env.push((k.into(), v.into()));
    }
    for (k, v) in child_env {
        config.env.push((k.into(), v.into()));
    }
    config.env_clear = true;
    add_runtime_spawn_env(&mut config, env);

    let mut child = match malt_platform::process::spawn(config) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: {e}\n");
            let code = match &e {
                malt_platform::process::SpawnError::NotFound { .. } => 127,
                malt_platform::process::SpawnError::PermissionDenied { .. } => 126,
                _ => 1,
            };
            return ExecResult::failure(code, msg);
        }
    };
    env.report_bg_pid(child.pid());

    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        if let Err(e) = out.read_to_end(&mut stdout_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stdout read failed: {e}\n").as_bytes(),
            );
        }
    }
    if let Some(mut err) = child.take_stderr() {
        if let Err(e) = err.read_to_end(&mut stderr_bytes) {
            stderr_bytes.extend_from_slice(
                format!("mash: {cmd_name}: stderr read failed: {e}\n").as_bytes(),
            );
        }
    }

    let exit_code = match wait_for_child_exit_code(&mut child, env) {
        Ok(code) => code,
        Err(e) => {
            let msg = format!("mash: {cmd_name}: wait failed: {e}\n");
            stderr_bytes.extend_from_slice(msg.as_bytes());
            1
        }
    };

    if merge_stderr_to_stdout {
        stdout_bytes.extend_from_slice(&stderr_bytes);
        stderr_bytes.clear();
    }
    if merge_stdout_to_stderr {
        stderr_bytes.extend_from_slice(&stdout_bytes);
        stdout_bytes.clear();
    }

    ExecResult {
        exit_code,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

fn is_exec_no_args_command(cmd_name: &str, argv: &[String]) -> bool {
    (cmd_name == "exec" && argv.is_empty())
        || (cmd_name == "command" && argv.len() == 1 && argv[0] == "exec")
}

/// Apply resolved redirects to an ExecResult: write stdout/stderr to files.
fn apply_output_redirects(result: &mut ExecResult, io: ResolvedIo) {
    // Apply natural-FD dup merges BEFORE writing to files.
    if io.stderr_to_stdout && io.stderr.is_none() {
        let stderr_bytes = std::mem::take(&mut result.stderr);
        result.stdout.extend_from_slice(&stderr_bytes);
    }
    if io.stdout_to_stderr && io.stdout.is_none() {
        let stdout_bytes = std::mem::take(&mut result.stdout);
        result.stderr.extend_from_slice(&stdout_bytes);
    }

    if let Some(mut stdout_file) = io.stdout {
        if let Err(e) = stdout_file.write_all(&result.stdout) {
            result
                .stderr
                .extend_from_slice(format!("mash: write error: {e}\n").as_bytes());
            result.exit_code = 1;
        }
        result.stdout = Vec::new();
    }
    if let Some(mut stderr_file) = io.stderr {
        let _ = stderr_file.write_all(&result.stderr);
        result.stderr = Vec::new();
    }
}

fn write_background_result(env: &Env, result: &ExecResult) {
    if !result.stdout.is_empty() {
        if let Ok(mut stdout) = env.open_fd_write(1) {
            let _ = stdout.write_all(&result.stdout);
        }
    }
    if !result.stderr.is_empty() {
        if let Ok(mut stderr) = env.open_fd_write(2) {
            let _ = stderr.write_all(&result.stderr);
        }
    }
}

fn handle_pending_background_signal(env: &mut Env, signal: &str, exit_code: i32) -> Option<i32> {
    match signal {
        "TSTP" | "CONT" => None,
        _ => {
            if let Some(trap) = env.get_trap(signal).cloned() {
                if trap.action.is_empty() {
                    return None;
                }
                let trap_result = execute_trap_action(&trap.action, env);
                if env.options().errexit && trap_result.exit_code != 0 {
                    env.request_exit(trap_result.exit_code);
                }
                env.exit_requested()
            } else {
                env.request_exit(exit_code);
                Some(exit_code)
            }
        }
    }
}

fn wait_for_child_exit_code(
    child: &mut malt_platform::process::Child,
    env: &mut Env,
) -> Result<i32, malt_platform::process::SpawnError> {
    loop {
        if let Some((signal, exit_code)) = env.take_pending_job_signal() {
            if let Some(code) = handle_pending_background_signal(env, &signal, exit_code) {
                return Ok(code);
            }
        }

        match child.try_wait()? {
            Some(status) => return Ok(status.code()),
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn execute_interruptible_sleep(argv: &[String], env: &mut Env) -> ExecResult {
    if argv.is_empty() {
        return ExecResult::failure(1, "sleep: missing operand\n");
    }

    let mut total_seconds = 0.0f64;
    for arg in argv {
        let seconds = match arg.parse::<f64>() {
            Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => seconds,
            _ => {
                return ExecResult::failure(
                    1,
                    format!("sleep: invalid time interval '{}'\n", arg),
                );
            }
        };
        total_seconds += seconds;
    }

    let deadline = Instant::now() + Duration::from_secs_f64(total_seconds);
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if let Some((signal, exit_code)) = env.take_pending_job_signal() {
            if let Some(code) = handle_pending_background_signal(env, &signal, exit_code) {
                return ExecResult::with_code(code);
            }
        }

        if remaining.is_zero() {
            break;
        }

        let step = remaining.min(Duration::from_millis(25));
        std::thread::sleep(step);
    }

    ExecResult::success()
}

fn apply_builtin_output_redirects(result: &mut ExecResult, io: ResolvedIo, builtin_name: &str) {
    if io.stderr_to_stdout && io.stderr.is_none() {
        let stderr_bytes = std::mem::take(&mut result.stderr);
        result.stdout.extend_from_slice(&stderr_bytes);
    }
    if io.stdout_to_stderr && io.stdout.is_none() {
        let stdout_bytes = std::mem::take(&mut result.stdout);
        result.stderr.extend_from_slice(&stdout_bytes);
    }

    if let Some(mut stdout_file) = io.stdout {
        if stdout_file.write_all(&result.stdout).is_err() {
            *result = builtin_output_io_error(builtin_name);
            return;
        }
        result.stdout = Vec::new();
    }
    if let Some(mut stderr_file) = io.stderr {
        if stderr_file.write_all(&result.stderr).is_err() {
            *result = builtin_output_io_error(builtin_name);
            return;
        }
        result.stderr = Vec::new();
    }
}

fn builtin_output_io_error(name: &str) -> ExecResult {
    let shell_name = if name == "times" { "smoosh" } else { "mash" };
    ExecResult::failure(2, format!("{shell_name}: {name}: I/O error\n"))
}

fn builtin_output_name(cmd_name: &str, argv: &[String]) -> String {
    if cmd_name == "command" {
        let mut iter = argv.iter().map(String::as_str).peekable();
        while let Some(arg) = iter.peek().copied() {
            match arg {
                "-p" => {
                    iter.next();
                }
                "-v" | "-V" => return cmd_name.to_string(),
                _ if arg.starts_with('-') => return cmd_name.to_string(),
                _ => break,
            }
        }
        if let Some(target) = iter.next() {
            if BUILTIN_NAMES.contains(&target) {
                return target.to_string();
            }
        }
    }
    cmd_name.to_string()
}

// ── PATH resolution ────────────────────────────────────────────────────

fn find_in_path(name: &str, env: &Env) -> Option<PathBuf> {
    // If name contains a path separator, use it directly.
    if name.contains('/') || name.contains('\\') {
        return Some(PathBuf::from(name));
    }

    for dir in shell_path_entries(env.get_str("PATH")) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(name);
        if is_path_search_match(&candidate) {
            return Some(candidate);
        }
        // On Windows, also check common executable extensions.
        #[cfg(windows)]
        {
            for ext in &["exe", "cmd", "bat", "com"] {
                let with_ext = candidate.with_extension(ext);
                if is_path_search_match(&with_ext) {
                    return Some(with_ext);
                }
            }
            if should_execute_shell_script_with_mash(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn resolve_source_path(name: &str, env: &Env) -> Option<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }

    if !env.options().sourcepath {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }

    for dir in shell_path_entries(env.get_str("PATH")) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() && malt_platform::fs::is_readable(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn shell_path_entries(path_var: &str) -> Vec<&str> {
    #[cfg(not(windows))]
    {
        path_var.split(':').collect()
    }

    #[cfg(windows)]
    {
        let bytes = path_var.as_bytes();
        let mut entries = Vec::new();
        let mut start = 0usize;

        for i in 0..bytes.len() {
            let byte = bytes[i];
            let is_separator = match byte {
                b';' => true,
                b':' => {
                    let prev = i.checked_sub(1).and_then(|idx| bytes.get(idx));
                    let next = bytes.get(i + 1);
                    !(matches!(prev, Some(b'a'..=b'z' | b'A'..=b'Z'))
                        && matches!(next, Some(b'/' | b'\\')))
                }
                _ => false,
            };

            if is_separator {
                entries.push(&path_var[start..i]);
                start = i + 1;
            }
        }

        entries.push(&path_var[start..]);
        entries
    }
}

fn is_path_search_match(path: &Path) -> bool {
    #[cfg(windows)]
    {
        path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "exe" | "cmd" | "bat" | "com"
                    )
                })
    }

    #[cfg(not(windows))]
    {
        path.is_file() && malt_platform::io::is_executable(path)
    }
}

#[cfg(test)]
mod tests {
    use super::find_in_path;
    use crate::env::{Env, Variable};

    #[test]
    #[cfg(windows)]
    fn find_in_path_prefers_executable_extension_over_bare_existing_file() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let dir = temp.path();
        let bare = dir.join("scr");
        let cmd = dir.join("scr.cmd");

        std::fs::write(&bare, "not executable").expect("write bare file");
        std::fs::write(&cmd, "@echo off\r\necho hi\r\n").expect("write cmd file");

        let mut env = Env::empty();
        env.set(
            "PATH",
            Variable::exported_string(dir.to_string_lossy().to_string()),
        )
        .expect("set PATH");

        let resolved = find_in_path("scr", &env).expect("resolve executable");
        assert_eq!(resolved, cmd);
    }

    #[test]
    #[cfg(windows)]
    fn find_in_path_accepts_colon_separated_shell_path_entries() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let dir = temp.path().join("bin");
        std::fs::create_dir_all(&dir).expect("create bin dir");
        let cmd = dir.join("scr.cmd");
        std::fs::write(&cmd, "@echo off\r\necho hi\r\n").expect("write cmd file");

        let mut env = Env::empty();
        env.set(
            "PATH",
            Variable::exported_string(format!("{}:C:/Windows/System32", dir.to_string_lossy())),
        )
        .expect("set PATH");

        let resolved = find_in_path("scr", &env).expect("resolve executable");
        assert_eq!(resolved, cmd);
    }
}
