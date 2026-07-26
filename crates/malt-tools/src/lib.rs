//! In-process POSIX utilities for MALT.
//!
//! This crate provides built-in implementations of common POSIX tools that
//! run natively within the shell process, avoiding fork/exec overhead for
//! frequent commands.
//!
//! All tools capture output into [`BuiltinResult`] buffers for shell pipeline
//! integration. The shell executor checks the [`Registry`] before falling back
//! to PATH lookup.

mod custom;

use std::collections::HashMap;

/// The result of running a built-in tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinResult {
    /// Exit code (0 = success).
    pub exit_code: i32,
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
}

impl BuiltinResult {
    /// Create a successful result with the given stdout.
    pub fn success(stdout: Vec<u8>) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: Vec::new(),
        }
    }

    /// Create a failure result with the given exit code and stderr.
    pub fn failure(code: i32, stderr: Vec<u8>) -> Self {
        Self {
            exit_code: code,
            stdout: Vec::new(),
            stderr,
        }
    }

    /// Returns `true` if the exit code is 0.
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

/// A tool implementation function.
///
/// Arguments: `(args, stdin, stdout)` -> `BuiltinResult`.
/// `stdin` is the byte content piped into the tool (empty slice if none).
///
/// `stdout` is where the tool's output is observable *while the tool is
/// still running* -- the mirror of feature 005's `stdin` reader change,
/// for the same reason: a tool that copies input to output (`cat`) must
/// make each piece of output visible before it reads the next piece of
/// input, not only after it has consumed all of it. A tool still returns
/// its complete output in `BuiltinResult::stdout` too, unconditionally --
/// that copy is what `apply_output_redirects` and pipeline capture use,
/// and it must be correct whether or not anything was streaming to
/// `stdout` (the caller supplies a no-op writer when this call's own
/// output is redirected or part of a pipeline). A tool that produces its
/// whole output at once (most of them) still writes it to `stdout` in one
/// call before returning -- there is no framing decision to make for
/// those, only for a tool that genuinely produces output incrementally
/// (`cat`, `grep`, `sed`, `head`, `wc`).
pub type ToolFn = fn(
    args: &[String],
    stdin: &mut dyn std::io::Read,
    stdout: &mut dyn std::io::Write,
) -> BuiltinResult;

/// Write a finished result's stdout to the streaming writer, then return the
/// result unchanged.
///
/// For a tool that builds its whole output before returning (most of them --
/// see the note on [`ToolFn`]), this is the one call that makes that output
/// observable while the command is still "running" from the caller's
/// perspective, without requiring the tool to restructure how it builds that
/// output. A tool with genuinely incremental output (`cat`, `grep`, `sed`,
/// `head`, `wc`) writes to `stdout` itself as it goes instead, and should not
/// also call this at the end -- that would write the same bytes twice.
pub fn emit(stdout: &mut dyn std::io::Write, result: BuiltinResult) -> BuiltinResult {
    if !result.stdout.is_empty() {
        let _ = stdout.write_all(&result.stdout);
    }
    result
}

/// Read a tool's entire standard input.
///
/// Correct for tools whose semantics really are "consume until end of input"
/// -- `cat`, `wc`, `sed`, `grep`. Against a terminal session's stdin this
/// blocks until the client signals end-of-input, which is exactly what a real
/// shell does; it is not a hang, and the cure is for the client to send EOF.
///
/// Tools that must stop before the end -- `head -n` is the obvious one --
/// must read incrementally instead of calling this, or they will wait for an
/// end that a live session has no reason to reach.
///
/// A read error yields whatever was read so far rather than being reported.
/// That matches the previous behaviour, where the caller pre-read the bytes
/// and a failure simply produced a shorter buffer.
pub fn read_all(stdin: &mut dyn std::io::Read) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = stdin.read_to_end(&mut buf);
    buf
}

/// Registry of in-process tools.
///
/// Pre-populated with all built-in POSIX utilities on construction via
/// [`Registry::new`].
pub struct Registry {
    tools: HashMap<String, ToolFn>,
}

impl Registry {
    /// Create a new registry pre-populated with all built-in tools.
    pub fn new() -> Self {
        let mut tools = HashMap::new();
        tools.insert("cat".to_string(), custom::cat::cat as ToolFn);
        tools.insert("chmod".to_string(), custom::chmod::chmod as ToolFn);
        tools.insert("cp".to_string(), custom::cp::cp as ToolFn);
        tools.insert("date".to_string(), custom::date::date as ToolFn);
        tools.insert("env".to_string(), custom::env::env_cmd as ToolFn);
        tools.insert("fds".to_string(), custom::fds::FDS);
        tools.insert("grep".to_string(), custom::grep::grep as ToolFn);
        tools.insert("head".to_string(), custom::head::head as ToolFn);
        tools.insert("ln".to_string(), custom::ln::ln as ToolFn);
        tools.insert("ls".to_string(), custom::ls::ls as ToolFn);
        tools.insert("mkdir".to_string(), custom::mkdir::mkdir as ToolFn);
        tools.insert("mv".to_string(), custom::mv::mv as ToolFn);
        tools.insert("rm".to_string(), custom::rm::rm as ToolFn);
        tools.insert("sed".to_string(), custom::sed::sed as ToolFn);
        tools.insert("sleep".to_string(), custom::sleep::sleep as ToolFn);
        tools.insert("touch".to_string(), custom::touch::touch as ToolFn);
        tools.insert("which".to_string(), custom::which::which as ToolFn);
        tools.insert("wc".to_string(), custom::wc::wc as ToolFn);
        Self { tools }
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<ToolFn> {
        self.tools.get(name).copied()
    }

    /// Returns `true` if the given name is a registered tool.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// All registered tool names, sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn registry_contains_sleep_tool() {
        let registry = Registry::new();
        assert!(registry.contains("sleep"));
    }

    #[test]
    fn registry_contains_date_tool() {
        let registry = Registry::new();
        assert!(registry.contains("date"));
    }

    #[test]
    fn date_tool_supports_unix_epoch_format() {
        let registry = Registry::new();
        let date = registry
            .get("date")
            .expect("date tool should be registered");
        let result = date(&["+%s".to_string()], &mut &b""[..], &mut std::io::sink());

        assert_eq!(result.exit_code, 0);
        assert!(result.stderr.is_empty());
        let output = String::from_utf8_lossy(&result.stdout);
        assert!(output.ends_with('\n'));
        assert!(
            output.trim_end().chars().all(|ch| ch.is_ascii_digit()),
            "unexpected date output: {output}"
        );
    }

    #[test]
    fn sleep_tool_waits_for_requested_duration() {
        let registry = Registry::new();
        let sleep = registry
            .get("sleep")
            .expect("sleep tool should be registered");
        let start = Instant::now();
        let result = sleep(&["0.05".to_string()], &mut &b""[..], &mut std::io::sink());
        let elapsed = start.elapsed();

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert!(
            elapsed >= Duration::from_millis(40),
            "sleep returned too quickly: {elapsed:?}"
        );
    }
}
