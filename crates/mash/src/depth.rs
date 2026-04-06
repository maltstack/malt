//! Depth limit tracking for MASH expansion and execution.
//!
//! This module provides thread-local counters for:
//! - Alias expansion depth (default limit: 1024)
//! - Subshell recursion depth (default limit: 256)

use std::cell::RefCell;

/// Default maximum alias expansion depth.
pub const DEFAULT_ALIAS_DEPTH_LIMIT: u32 = 1024;

/// Default maximum subshell recursion depth.
pub const DEFAULT_SUBSHELL_DEPTH_LIMIT: u32 = 256;

/// Error type for depth limit violations.
#[derive(Debug, Clone, PartialEq)]
pub enum DepthError {
    /// Alias expansion exceeded the depth limit.
    AliasLimitExceeded { limit: u32 },
    /// Subshell recursion exceeded the depth limit.
    SubshellLimitExceeded { limit: u32 },
}

impl std::fmt::Display for DepthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DepthError::AliasLimitExceeded { limit } => {
                write!(f, "alias expansion depth limit exceeded ({limit})")
            }
            DepthError::SubshellLimitExceeded { limit } => {
                write!(f, "subshell recursion depth limit exceeded ({limit})")
            }
        }
    }
}

impl std::error::Error for DepthError {}

thread_local! {
    /// Thread-local depth counter for alias expansion.
    static ALIAS_DEPTH: RefCell<u32> = const { RefCell::new(0) };
    /// Thread-local depth counter for subshell recursion.
    static SUBSHELL_DEPTH: RefCell<u32> = const { RefCell::new(0) };
}

/// Increment the alias expansion depth counter.
/// Returns an error if the limit would be exceeded.
pub fn enter_alias_expansion() -> Result<(), DepthError> {
    ALIAS_DEPTH.with(|depth| {
        let mut d = depth.borrow_mut();
        if *d >= DEFAULT_ALIAS_DEPTH_LIMIT {
            Err(DepthError::AliasLimitExceeded {
                limit: DEFAULT_ALIAS_DEPTH_LIMIT,
            })
        } else {
            *d += 1;
            Ok(())
        }
    })
}

/// Decrement the alias expansion depth counter.
pub fn exit_alias_expansion() {
    ALIAS_DEPTH.with(|depth| {
        let mut d = depth.borrow_mut();
        if *d > 0 {
            *d -= 1;
        }
    });
}

/// Get the current alias expansion depth.
pub fn current_alias_depth() -> u32 {
    ALIAS_DEPTH.with(|depth| *depth.borrow())
}

/// Reset the alias expansion depth counter to zero.
pub fn reset_alias_depth() {
    ALIAS_DEPTH.with(|depth| *depth.borrow_mut() = 0);
}

/// Increment the subshell recursion depth counter.
/// Returns an error if the limit would be exceeded.
pub fn enter_subshell() -> Result<(), DepthError> {
    SUBSHELL_DEPTH.with(|depth| {
        let mut d = depth.borrow_mut();
        if *d >= DEFAULT_SUBSHELL_DEPTH_LIMIT {
            Err(DepthError::SubshellLimitExceeded {
                limit: DEFAULT_SUBSHELL_DEPTH_LIMIT,
            })
        } else {
            *d += 1;
            Ok(())
        }
    })
}

/// Decrement the subshell recursion depth counter.
pub fn exit_subshell() {
    SUBSHELL_DEPTH.with(|depth| {
        let mut d = depth.borrow_mut();
        if *d > 0 {
            *d -= 1;
        }
    });
}

/// Get the current subshell recursion depth.
pub fn current_subshell_depth() -> u32 {
    SUBSHELL_DEPTH.with(|depth| *depth.borrow())
}

/// Reset the subshell recursion depth counter to zero.
pub fn reset_subshell_depth() {
    SUBSHELL_DEPTH.with(|depth| *depth.borrow_mut() = 0);
}

/// Reset all depth counters to zero.
/// Called when starting a new command or after a panic recovery.
pub fn reset_all_depths() {
    reset_alias_depth();
    reset_subshell_depth();
}

/// A guard that automatically decrements alias depth when dropped.
pub struct AliasExpansionGuard;

impl AliasExpansionGuard {
    /// Enter alias expansion, returning a guard that will exit on drop.
    pub fn enter() -> Result<Self, DepthError> {
        enter_alias_expansion()?;
        Ok(Self)
    }
}

impl Drop for AliasExpansionGuard {
    fn drop(&mut self) {
        exit_alias_expansion();
    }
}

/// A guard that automatically decrements subshell depth when dropped.
pub struct SubshellGuard;

impl SubshellGuard {
    /// Enter subshell, returning a guard that will exit on drop.
    pub fn enter() -> Result<Self, DepthError> {
        enter_subshell()?;
        Ok(Self)
    }
}

impl Drop for SubshellGuard {
    fn drop(&mut self) {
        exit_subshell();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_depth_tracks_nesting() {
        reset_alias_depth();
        assert_eq!(current_alias_depth(), 0);

        enter_alias_expansion().unwrap();
        assert_eq!(current_alias_depth(), 1);

        enter_alias_expansion().unwrap();
        assert_eq!(current_alias_depth(), 2);

        exit_alias_expansion();
        assert_eq!(current_alias_depth(), 1);

        exit_alias_expansion();
        assert_eq!(current_alias_depth(), 0);
    }

    #[test]
    fn alias_guard_auto_decrements() {
        reset_alias_depth();
        assert_eq!(current_alias_depth(), 0);

        {
            let _guard = AliasExpansionGuard::enter().unwrap();
            assert_eq!(current_alias_depth(), 1);
        }

        assert_eq!(current_alias_depth(), 0);
    }

    #[test]
    fn alias_depth_limit_enforced() {
        reset_alias_depth();

        // Enter up to the limit
        for _ in 0..DEFAULT_ALIAS_DEPTH_LIMIT {
            enter_alias_expansion().unwrap();
        }

        // Next entry should fail
        let result = enter_alias_expansion();
        assert!(matches!(
            result,
            Err(DepthError::AliasLimitExceeded { limit: 1024 })
        ));
    }

    #[test]
    fn subshell_depth_tracks_nesting() {
        reset_subshell_depth();
        assert_eq!(current_subshell_depth(), 0);

        enter_subshell().unwrap();
        assert_eq!(current_subshell_depth(), 1);

        exit_subshell();
        assert_eq!(current_subshell_depth(), 0);
    }

    #[test]
    fn subshell_depth_limit_enforced() {
        reset_subshell_depth();

        // Enter up to the limit
        for _ in 0..DEFAULT_SUBSHELL_DEPTH_LIMIT {
            enter_subshell().unwrap();
        }

        // Next entry should fail
        let result = enter_subshell();
        assert!(matches!(
            result,
            Err(DepthError::SubshellLimitExceeded { limit: 256 })
        ));
    }

    #[test]
    fn reset_all_clears_both() {
        enter_alias_expansion().unwrap();
        enter_subshell().unwrap();

        assert_eq!(current_alias_depth(), 1);
        assert_eq!(current_subshell_depth(), 1);

        reset_all_depths();

        assert_eq!(current_alias_depth(), 0);
        assert_eq!(current_subshell_depth(), 0);
    }
}
