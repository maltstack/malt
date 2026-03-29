//! Executor — walks the AST and runs commands.
//!
//! This is the scaffold: simple external commands, AND-OR lists, brace groups,
//! env assignments. Pipelines, redirects, control flow, and builtins are
//! added in subsequent tasks.

use std::io::Read as IoRead;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use crate::ast::{Command, ListOp, Redirect, RedirectKind, Span, Spanned};
use crate::env::{CallFrame, Env, LoopControl, Variable};
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
        Self { exit_code: 0, stdout: Vec::new(), stderr: Vec::new() }
    }

    fn with_code(code: i32) -> Self {
        Self { exit_code: code, stdout: Vec::new(), stderr: Vec::new() }
    }

    fn failure(code: i32, msg: impl Into<String>) -> Self {
        Self {
            exit_code: code,
            stdout: Vec::new(),
            stderr: msg.into().into_bytes(),
        }
    }
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

        let result = execute(cmd, source, env);
        last_code = result.exit_code;
        all_stdout.extend_from_slice(&result.stdout);
        all_stderr.extend_from_slice(&result.stderr);

        // set -e (errexit): abort on non-zero exit code.
        if env.options().errexit && last_code != 0 {
            env.request_exit(last_code);
            break;
        }
    }
    ExecResult {
        exit_code: last_code,
        stdout: all_stdout,
        stderr: all_stderr,
    }
}

/// Execute a command string and capture stdout. Used by expander for $(cmd).
/// Runs in a cloned Env (subshell semantics). Strips trailing newlines.
pub fn capture_command(cmd_str: &str, env: &mut Env) -> Result<String, crate::expander::ExpandError> {
    let cmds = match crate::parser::parse(cmd_str) {
        Ok(cmds) => cmds,
        Err(e) => return Err(crate::expander::ExpandError::CommandSubstitution(e.to_string())),
    };
    let mut sub_env = env.clone();
    let result = execute_list(&cmds, cmd_str, &mut sub_env);
    env.set_exit_code(result.exit_code);
    let mut output = String::from_utf8_lossy(&result.stdout).to_string();
    // Strip trailing newlines (POSIX).
    while output.ends_with('\n') { output.pop(); }
    while output.ends_with('\r') { output.pop(); }
    Ok(output)
}

// ── Dispatch ───────────────────────────────────────────────────────────

fn execute_inner(cmd: &Spanned<Command>, source: &str, env: &mut Env) -> ExecResult {
    match &cmd.node {
        Command::Empty => ExecResult::success(),

        Command::Simple { name, args, redirects, env_assigns } => {
            execute_simple(name, args, redirects, env_assigns, source, env)
        }

        Command::EnvAssign { assigns } => {
            execute_env_assign(assigns, source, env)
        }

        Command::List { pairs, last } => {
            execute_list_node(pairs, last, source, env)
        }

        Command::BraceGroup { body } => {
            execute_list(body, source, env)
        }

        Command::Subshell { body } => {
            // Clone env so changes in the subshell don't affect the parent.
            let mut sub_env = env.clone();
            let result = execute_list(body, source, &mut sub_env);
            // Propagate exit code but not env changes.
            result
        }

        Command::Pipeline { commands, negated } => {
            execute_pipeline(commands, *negated, source, env)
        }

        Command::Background(inner) => {
            // Spawn in a thread with a cloned env. Real PID tracking comes with job control.
            static BG_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
            let bg_id = BG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut bg_env = env.clone();
            let bg_source = source.to_string();
            let bg_cmd = inner.as_ref().clone();
            std::thread::spawn(move || {
                execute(&bg_cmd, &bg_source, &mut bg_env);
            });
            env.set_last_bg_pid(bg_id);
            ExecResult::success()
        }

        Command::Redirected { cmd: inner, redirects } => {
            let resolved_io = match resolve_redirects(redirects, source, env) {
                Ok(io) => io,
                Err(err_result) => return err_result,
            };
            // Execute the inner command, capturing its output.
            let mut result = execute(inner, source, env);
            // Apply redirects: write captured output to redirect files.
            apply_output_redirects(&mut result, resolved_io);
            result
        }

        Command::If { condition, then_body, elif_clauses, else_body } => {
            execute_if(condition, then_body, elif_clauses, else_body.as_deref(), source, env)
        }

        Command::While { condition, body } => {
            execute_while_until(condition, body, /* is_until */ false, source, env)
        }

        Command::Until { condition, body } => {
            execute_while_until(condition, body, /* is_until */ true, source, env)
        }

        Command::For { var, words, body } => {
            execute_for(var, words, body, source, env)
        }

        Command::ForArith { init, cond, step, body } => {
            execute_for_arith(init, cond, step, body, source, env)
        }

        Command::Case { word, items } => {
            execute_case(word, items, source, env)
        }

        Command::Select { var, words, body } => {
            // Select requires interactive input. Stub with code 1.
            let _ = (var, words, body);
            ExecResult::failure(1, "mash: select: not yet implemented (requires interactive input)\n")
        }

        Command::FunctionDef { name, body } => {
            let func_name = name.text(source).to_string();
            // Store the full original source so body spans remain valid.
            env.define_function(func_name, source.to_string(), body.as_ref().clone());
            ExecResult::success()
        }

        Command::Arithmetic { expr } => {
            execute_arithmetic(expr, source, env)
        }

        Command::Conditional { expr } => {
            execute_conditional(expr, source, env)
        }

        Command::Coproc { name, cmd: inner } => {
            let _ = name;
            // Coproc requires bidirectional pipe management. Stub.
            let result = execute(inner, source, env);
            result
        }

        Command::Time { posix_format, command } => {
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
        let stage_source = source.to_string();
        let stage_cmd = commands[i].clone();

        // stdin for this stage: stage 0 inherits, others read from pipe[i-1].
        let stdin_file: Option<std::fs::File> = if i > 0 {
            read_ends[i - 1].take()
        } else {
            None
        };

        // stdout for this stage: last stage captures (Pipe), others write to pipe[i].
        let stdout_file: Option<std::fs::File> = if i < n - 1 {
            write_ends[i].take()
        } else {
            None
        };

        handles.push(std::thread::spawn(move || {
            execute_with_io(&stage_cmd, &stage_source, &mut stage_env, stdin_file, stdout_file)
        }));
    }

    // 3. Drop remaining parent-side pipe ends so stages see EOF.
    drop(read_ends);
    drop(write_ends);

    // 4. Join all threads, collect results.
    let results: Vec<ExecResult> = handles
        .into_iter()
        .map(|h| h.join().unwrap_or_else(|_| ExecResult::with_code(1)))
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
        Command::Simple { name, args, redirects, env_assigns } => {
            execute_simple_with_io(
                name, args, redirects, env_assigns,
                source, env,
                stdin_file, stdout_file,
            )
        }
        _ => {
            // For non-simple commands (brace groups, subshells, etc.),
            // execute normally and then write captured stdout to the pipe.
            let mut result = execute(cmd, source, env);

            // If stdin was provided but we can't wire it into a non-simple
            // command, we just drop it (data goes to /dev/null effectively).
            drop(stdin_file);

            // Write captured stdout to the pipe fd.
            if let Some(mut pipe_out) = stdout_file {
                let _ = pipe_out.write_all(&result.stdout);
                result.stdout = Vec::new();
            }

            result
        }
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
    stdin_file: Option<std::fs::File>,
    stdout_file: Option<std::fs::File>,
) -> ExecResult {
    // 1. Expand command name.
    let name_text = name_span.text(source);
    let expanded_name = match expander::expand_word(name_text, env) {
        Ok(fields) => fields,
        Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
    };
    let cmd_name = match expanded_name.first() {
        Some(n) if !n.is_empty() => n.clone(),
        _ => return ExecResult::success(),
    };

    // 2. Expand arguments.
    let mut argv: Vec<String> = Vec::new();
    argv.extend(expanded_name.into_iter().skip(1));
    for arg_span in arg_spans {
        let arg_text = arg_span.text(source);
        match expander::expand_word(arg_text, env) {
            Ok(fields) => argv.extend(fields),
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        }
    }

    // 3. Handle builtins in pipeline context.
    if let Some(mut result) = try_execute_builtin(&cmd_name, &argv, env) {
        drop(stdin_file);
        if let Some(mut pipe_out) = stdout_file {
            let _ = pipe_out.write_all(&result.stdout);
            result.stdout = Vec::new();
        }
        return result;
    }

    // 4. Temporary env assignments.
    let mut child_env: Vec<(String, String)> = Vec::new();
    for (key_span, val_span) in env_assigns {
        let key = key_span.text(source).to_string();
        let val_text = val_span.text(source);
        let val = match expander::expand_word_nosplit(val_text, env) {
            Ok(v) => v,
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        };
        child_env.push((key, val));
    }

    // 4. Resolve explicit redirects (these override pipeline I/O).
    let resolved_io = match resolve_redirects(redirects, source, env) {
        Ok(io) => io,
        Err(err_result) => return err_result,
    };

    // 5. Check for shell functions (in pipeline context).
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
        // Write captured stdout to pipeline.
        drop(stdin_file);
        if let Some(mut pipe_out) = stdout_file {
            let _ = pipe_out.write_all(&result.stdout);
            result.stdout = Vec::new();
        }
        return result;
    }

    // 6. Resolve executable path.
    let program = match find_in_path(&cmd_name, env) {
        Some(p) => p,
        None => {
            let msg = format!("mash: {cmd_name}: command not found\n");
            return ExecResult::failure(127, msg);
        }
    };

    // 7. Build SpawnConfig with pipeline I/O + redirect overrides.
    let mut config = malt_platform::process::SpawnConfig::new(&program);
    config.args = argv.iter().map(|a| a.into()).collect();

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

    // Read captured output.
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        let _ = out.read_to_end(&mut stdout_bytes);
    }
    if let Some(mut err) = child.take_stderr() {
        let _ = err.read_to_end(&mut stderr_bytes);
    }

    // Wait.
    let exit_code = match child.wait() {
        Ok(status) => status.code(),
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
        Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
    };
    let cmd_name = match expanded_name.first() {
        Some(n) if !n.is_empty() => n.clone(),
        _ => return ExecResult::success(), // Null command.
    };

    // 2. Expand all arguments.
    let mut argv: Vec<String> = Vec::new();
    // Include remaining fields from name expansion (if word split produced multiple).
    argv.extend(expanded_name.into_iter().skip(1));
    for arg_span in arg_spans {
        let arg_text = arg_span.text(source);
        match expander::expand_word(arg_text, env) {
            Ok(fields) => argv.extend(fields),
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        }
    }

    // 3. Collect temporary env assignments for the child process.
    let mut child_env: Vec<(String, String)> = Vec::new();
    for (key_span, val_span) in env_assigns {
        let key = key_span.text(source).to_string();
        let val_text = val_span.text(source);
        let val = match expander::expand_word_nosplit(val_text, env) {
            Ok(v) => v,
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        };
        child_env.push((key, val));
    }

    // 4. Resolve redirects.
    let resolved_io = match resolve_redirects(redirects, source, env) {
        Ok(io) => io,
        Err(err_result) => return err_result,
    };

    // 5. Handle special builtins (break, continue, return, true, false, exit, :, echo).
    if let Some(mut result) = try_execute_builtin(&cmd_name, &argv, env) {
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

        // Apply temporary env assignments in function scope.
        for (k, v) in &child_env {
            let _ = env.set(k, Variable::string(v.clone()));
        }

        // Execute function body using the stored source (spans reference it).
        let stored_source = func_def.source.clone();
        let func_body = func_def.body.clone();
        let mut result = execute(&func_body, &stored_source, env);

        // Handle return.
        if let LoopControl::Return(code) = env.loop_control().clone() {
            result.exit_code = code;
            env.set_loop_control(LoopControl::None);
        }

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

    // 7. Build SpawnConfig and execute.
    let mut config = malt_platform::process::SpawnConfig::new(&program);
    config.args = argv.iter().map(|a| a.into()).collect();

    // Apply redirect files to the spawn config.
    config.stdin = match resolved_io.stdin {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Inherit,
    };
    config.stdout = match resolved_io.stdout {
        Some(f) => malt_platform::process::Io::File(f),
        None => malt_platform::process::Io::Pipe, // Capture for ExecResult.
    };
    config.stderr = match resolved_io.stderr {
        Some(f) => malt_platform::process::Io::File(f),
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

    // Read stdout and stderr (only populated when fd is Pipe, not File).
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    if let Some(mut out) = child.take_stdout() {
        let _ = out.read_to_end(&mut stdout_bytes);
    }
    if let Some(mut err) = child.take_stderr() {
        let _ = err.read_to_end(&mut stderr_bytes);
    }

    // Wait for the child.
    let exit_code = match child.wait() {
        Ok(status) => status.code(),
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

// ── Builtin commands (minimal set for control flow) ───────────────────

/// Try to execute a builtin command. Returns None if not a builtin.
fn try_execute_builtin(cmd_name: &str, argv: &[String], env: &mut Env) -> Option<ExecResult> {
    match cmd_name {
        "break" => {
            let n: usize = argv.first().and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
            env.set_loop_control(LoopControl::Break(n));
            Some(ExecResult::success())
        }
        "continue" => {
            let n: usize = argv.first().and_then(|s| s.parse().ok()).unwrap_or(1).max(1);
            env.set_loop_control(LoopControl::Continue(n));
            Some(ExecResult::success())
        }
        "return" => {
            let code: i32 = argv.first().and_then(|s| s.parse().ok()).unwrap_or(env.exit_code());
            env.set_loop_control(LoopControl::Return(code));
            Some(ExecResult::with_code(code))
        }
        "exit" => {
            let code: i32 = argv.first().and_then(|s| s.parse().ok()).unwrap_or(env.exit_code());
            env.request_exit(code);
            Some(ExecResult::with_code(code))
        }
        "true" | ":" => {
            Some(ExecResult::success())
        }
        "false" => {
            Some(ExecResult::with_code(1))
        }
        "echo" => {
            // Built-in echo to avoid depending on external echo binary.
            let mut output = argv.join(" ");
            output.push('\n');
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
                                        "noclobber" => env.options_mut().noclobber = true,
                                        "noexec" => env.options_mut().noexec = true,
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
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
                                        "noclobber" => env.options_mut().noclobber = false,
                                        "noexec" => env.options_mut().noexec = false,
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
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
            let file = match argv.first() {
                Some(f) => f,
                None => return Some(ExecResult::with_code(2)),
            };
            let contents = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    return Some(ExecResult {
                        exit_code: 1,
                        stdout: Vec::new(),
                        stderr: format!("source: {}: {}\n", file, e).into_bytes(),
                    });
                }
            };
            match crate::parser::parse(&contents) {
                Ok(cmds) => Some(execute_list(&cmds, &contents, env)),
                Err(e) => Some(ExecResult {
                    exit_code: 1,
                    stdout: Vec::new(),
                    stderr: format!("source: parse error: {}\n", e).into_bytes(),
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
                            errors.extend_from_slice(
                                format!("mash: unset: {}\n", e).as_bytes(),
                            );
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
                if arg == "-p" {
                    continue;
                }
                if let Some((name, val)) = arg.split_once('=') {
                    if env.get(name).is_some_and(|v| v.readonly) {
                        errors.extend_from_slice(
                            format!("mash: readonly: {}: readonly variable\n", name).as_bytes(),
                        );
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
                            { s.replace('\\', "/") }
                            #[cfg(not(windows))]
                            { s }
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
                Err(e) => {
                    Some(ExecResult::failure(1, format!("mash: cd: {}: {}\n", target, e)))
                }
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
                        { s.replace('\\', "/") }
                        #[cfg(not(windows))]
                        { s }
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
                            { s.replace('\\', "/") }
                            #[cfg(not(windows))]
                            { s }
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
        _ => None,
    }
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
    let prev_errexit = env.options().errexit;
    env.options_mut().errexit = false;
    let cond_result = execute(condition, source, env);
    env.options_mut().errexit = prev_errexit;
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
        env.options_mut().errexit = false;
        let elif_result = execute(elif_cond, source, env);
        env.options_mut().errexit = prev_errexit;
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

    ExecResult { exit_code: 0, stdout: all_stdout, stderr: all_stderr }
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
        let prev_errexit = env.options().errexit;
        env.options_mut().errexit = false;
        let cond_result = execute(condition, source, env);
        env.options_mut().errexit = prev_errexit;

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
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
                    env.set_loop_control(LoopControl::None);
                    // Continue to next iteration.
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
    ExecResult { exit_code: last_code, stdout: all_stdout, stderr: all_stderr }
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
            return ExecResult::failure(1, format!("mash: {e}\n"));
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
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
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
    ExecResult { exit_code: last_code, stdout: all_stdout, stderr: all_stderr }
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
                } else {
                    env.set_loop_control(LoopControl::Break(n - 1));
                }
                break;
            }
            LoopControl::Continue(n) => {
                if n <= 1 {
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
    ExecResult { exit_code: last_code, stdout: all_stdout, stderr: all_stderr }
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
    if result { ExecResult::success() } else { ExecResult::with_code(1) }
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
            let right = eval_conditional_tokens(&tokens[i+1..], env);
            return left && right;
        }
        if tokens[i] == "||" {
            let left = eval_conditional_tokens(&tokens[..i], env);
            let right = eval_conditional_tokens(&tokens[i+1..], env);
            return left || right;
        }
    }

    // Handle parenthesized expressions.
    if tokens.first() == Some(&"(") && tokens.last() == Some(&")") {
        return eval_conditional_tokens(&tokens[1..tokens.len()-1], env);
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
        s[1..s.len()-1].to_string()
    } else {
        s.to_string()
    }
}

// ── Env assignment ─────────────────────────────────────────────────────

fn execute_env_assign(
    assigns: &[(Span, Span)],
    source: &str,
    env: &mut Env,
) -> ExecResult {
    for (key_span, val_span) in assigns {
        let key = key_span.text(source);
        let val_text = val_span.text(source);
        let val = match expander::expand_word_nosplit(val_text, env) {
            Ok(v) => v,
            Err(e) => return ExecResult::failure(1, format!("mash: {e}\n")),
        };
        if let Err(e) = env.set(key, Variable::string(val)) {
            return ExecResult::failure(1, format!("mash: {e}\n"));
        }
    }
    ExecResult::success()
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

        match op {
            ListOp::Sequential => {
                // Always continue.
            }
            ListOp::Background => {
                // Scaffold: just continue (true background comes later).
            }
            ListOp::AndIf => {
                if result.exit_code != 0 {
                    // Short-circuit: skip the rest until we see OrIf or Sequential.
                    // But since the AST pairs are flat, we need to skip to `last`.
                    // Actually the parser structures AND-OR so each pair is one link.
                    // If the pair fails on AndIf, the next command is `last` which
                    // we should skip. Return current result.
                    result.stdout = all_stdout;
                    result.stderr = all_stderr;
                    return result;
                }
            }
            ListOp::OrIf => {
                if result.exit_code == 0 {
                    // Short-circuit on success for OrIf.
                    result.stdout = all_stdout;
                    result.stderr = all_stderr;
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
    result
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
}

impl ResolvedIo {
    fn new() -> Self {
        Self {
            stdin: None,
            stdout: None,
            stderr: None,
            stdout_to_stderr: false,
            stderr_to_stdout: false,
        }
    }
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

    for redir_spanned in redirects {
        let redirect = &redir_spanned.node;

        // Default fd: 0 for input-like redirects, 1 for output-like.
        let fd: u8 = redirect.fd.map(|f| f as u8).unwrap_or(match redirect.kind {
            RedirectKind::Input
            | RedirectKind::InputOutput
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::DupInput => 0,
            _ => 1,
        });

        // Expand the target. For heredoc/herestring the target span contains
        // the body text directly.
        let raw_target = redirect.target.text(source);

        // For heredoc kinds, expand the body if not quoted.
        let target: String = match redirect.kind {
            RedirectKind::HereDoc | RedirectKind::HereDocStrip => {
                if redirect.quoted {
                    raw_target.to_string()
                } else {
                    match expander::expand_heredoc_body(raw_target, env) {
                        Ok(s) => s,
                        Err(e) => {
                            let msg = format!("mash: heredoc expansion: {e}\n");
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

        let mut assign_fd = |fd: u8, file: std::fs::File| match fd {
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
                // Extra fds not yet supported; drop.
            }
        };

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
                let file = std::fs::File::create(target).map_err(|e| {
                    ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                })?;
                assign_fd(fd, file);
            }
            RedirectKind::Append => {
                use std::io::Seek;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(target)
                    .map_err(|e| {
                        ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                    })?;
                // Seek to end so writes append. Using write+seek instead of
                // append mode avoids issues with MSYS2 binaries on Windows.
                let _ = file.seek(std::io::SeekFrom::End(0));
                assign_fd(fd, file);
            }
            RedirectKind::Input => {
                let file = std::fs::File::open(target).map_err(|e| {
                    ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                })?;
                assign_fd(fd, file);
            }
            RedirectKind::HereDoc | RedirectKind::HereDocStrip | RedirectKind::HereString => {
                let (read, mut write) = malt_platform::io::create_pipe().map_err(|e| {
                    ExecResult::failure(1, format!("mash: pipe: {e}\n"))
                })?;
                let data = if redirect.kind == RedirectKind::HereString {
                    format!("{target}\n")
                } else {
                    target.to_string()
                };
                // Write in a thread to avoid blocking if pipe buffer fills.
                std::thread::spawn(move || {
                    let _ = write.write_all(data.as_bytes());
                });
                assign_fd(fd, read);
            }
            RedirectKind::Both => {
                // &> file — redirect both stdout and stderr.
                let file = std::fs::File::create(target).map_err(|e| {
                    ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                })?;
                let file2 = file.try_clone().map_err(|e| {
                    ExecResult::failure(1, format!("mash: {target}: clone: {e}\n"))
                })?;
                io.stdout = Some(file);
                io.stderr = Some(file2);
                io.stdout_to_stderr = false;
                io.stderr_to_stdout = false;
            }
            RedirectKind::InputOutput => {
                // <> file — open for reading and writing.
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(target)
                    .map_err(|e| {
                        ExecResult::failure(1, format!("mash: {target}: {e}\n"))
                    })?;
                assign_fd(fd, file);
            }
            RedirectKind::DupInput | RedirectKind::DupOutput => {
                if target == "-" {
                    // Close fd.
                    match fd {
                        0 => io.stdin = None,
                        1 => io.stdout = None,
                        2 => io.stderr = None,
                        _ => {}
                    }
                } else {
                    let src: u8 = target.parse::<u8>().unwrap_or(
                        if redirect.kind == RedirectKind::DupInput { 0 } else { 1 },
                    );
                    let cloned_file = match src {
                        0 => io.stdin.as_ref().and_then(|f| f.try_clone().ok()),
                        1 => io.stdout.as_ref().and_then(|f| f.try_clone().ok()),
                        2 => io.stderr.as_ref().and_then(|f| f.try_clone().ok()),
                        _ => None,
                    };
                    match fd {
                        0 => io.stdin = cloned_file,
                        1 => {
                            if cloned_file.is_none() && src == 2 {
                                io.stdout_to_stderr = true;
                                io.stdout = None;
                            } else {
                                io.stdout = cloned_file;
                                if io.stdout.is_some() {
                                    io.stdout_to_stderr = false;
                                }
                            }
                        }
                        2 => {
                            if cloned_file.is_none() && src == 1 {
                                io.stderr_to_stdout = true;
                                io.stderr = None;
                            } else {
                                io.stderr = cloned_file;
                                if io.stderr.is_some() {
                                    io.stderr_to_stdout = false;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(io)
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

// ── PATH resolution ────────────────────────────────────────────────────

fn find_in_path(name: &str, env: &Env) -> Option<PathBuf> {
    // If name contains a path separator, use it directly.
    if name.contains('/') || name.contains('\\') {
        return Some(PathBuf::from(name));
    }

    let path_var = env.get_str("PATH");
    let separator = if cfg!(windows) { ';' } else { ':' };

    for dir in path_var.split(separator) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        // On Windows, also check common executable extensions.
        #[cfg(windows)]
        {
            for ext in &["exe", "cmd", "bat", "com"] {
                let with_ext = candidate.with_extension(ext);
                if with_ext.exists() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}
