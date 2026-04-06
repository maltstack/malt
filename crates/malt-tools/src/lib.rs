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
/// Arguments: `(args, stdin)` -> `BuiltinResult`.
/// `stdin` is the byte content piped into the tool (empty slice if none).
pub type ToolFn = fn(args: &[String], stdin: &[u8]) -> BuiltinResult;

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
        tools.insert("cp".to_string(), custom::cp::cp as ToolFn);
        tools.insert("env".to_string(), custom::env::env_cmd as ToolFn);
        tools.insert("grep".to_string(), custom::grep::grep as ToolFn);
        tools.insert("head".to_string(), custom::head::head as ToolFn);
        tools.insert("ls".to_string(), custom::ls::ls as ToolFn);
        tools.insert("mkdir".to_string(), custom::mkdir::mkdir as ToolFn);
        tools.insert("mv".to_string(), custom::mv::mv as ToolFn);
        tools.insert("rm".to_string(), custom::rm::rm as ToolFn);
        tools.insert("sed".to_string(), custom::sed::sed as ToolFn);
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
