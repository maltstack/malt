//! Builtin command registry for MASH.
//!
//! This module defines the trait and registry for shell builtins.
//! Actual builtin implementations will be added incrementally.

use crate::env::Env;
use crate::executor::ExecResult;
use std::collections::HashMap;

/// A shell builtin command.
pub trait Builtin: Send + Sync {
    /// The name of the builtin (e.g., "cd", "export").
    fn name(&self) -> &str;

    /// Execute the builtin with the given arguments.
    fn execute(&self, args: &[String], env: &mut Env) -> ExecResult;

    /// Whether this is a POSIX special builtin.
    ///
    /// Special builtins (break, continue, return, exit, export, readonly, set, etc.)
    /// have different error handling semantics: errors in special builtins
    /// cause the shell to exit in non-interactive mode.
    fn is_special(&self) -> bool;
}

/// Registry mapping builtin names to their implementations.
pub struct BuiltinRegistry {
    builtins: HashMap<String, Box<dyn Builtin>>,
}

impl BuiltinRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            builtins: HashMap::new(),
        }
    }

    /// Look up a builtin by name.
    pub fn get(&self, name: &str) -> Option<&dyn Builtin> {
        self.builtins.get(name).map(|b| b.as_ref())
    }

    /// Register a builtin command.
    pub fn register(&mut self, builtin: Box<dyn Builtin>) {
        let name = builtin.name().to_string();
        self.builtins.insert(name, builtin);
    }
}

impl Default for BuiltinRegistry {
    fn default() -> Self {
        Self::new()
    }
}
