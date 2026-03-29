//! Executor — walks the AST and runs commands.
//!
//! This is the scaffold: simple external commands, AND-OR lists, brace groups,
//! env assignments. Pipelines, redirects, control flow, and builtins are
//! added in subsequent tasks.

use std::io::Read as IoRead;
use std::io::Write as IoWrite;
use std::path::PathBuf;

use crate::ast::{Command, ListOp, Redirect, RedirectKind, Span, Spanned};
use crate::env::{Env, LoopControl, Variable};
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
            // For now, execute in the same env (true subshell isolation comes later).
            execute_list(body, source, env)
        }

        Command::Pipeline { commands, negated } => {
            // Scaffold: run commands sequentially, pass last stdout as first stdin.
            // True pipeline (pipes between processes) comes in Task 3.
            let mut result = ExecResult::success();
            for c in commands {
                result = execute(c, source, env);
            }
            if *negated {
                result.exit_code = if result.exit_code == 0 { 1 } else { 0 };
            }
            result
        }

        Command::Background(inner) => {
            // Scaffold: just execute synchronously, background spawning comes later.
            let result = execute(inner, source, env);
            result
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

        // Not yet implemented variants return 127.
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

    // 5. Resolve the executable path.
    let program = match find_in_path(&cmd_name, env) {
        Some(p) => p,
        None => {
            let msg = format!("mash: {cmd_name}: command not found\n");
            return ExecResult::failure(127, msg);
        }
    };

    // 6. Build SpawnConfig and execute.
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
