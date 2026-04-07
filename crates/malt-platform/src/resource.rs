//! Resource usage querying — CPU times, memory, etc.
//!
//! Provides `get_rusage()` for POSIX `getrusage(RUSAGE_SELF/RUSAGE_CHILDREN)`
//! emulation. On Windows, this returns zeros since the syscall is unavailable.

/// Resource usage statistics for the shell and its children.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceUsage {
    /// User CPU time in seconds (self).
    pub user_secs: f64,
    /// System CPU time in seconds (self).
    pub sys_secs: f64,
    /// User CPU time in seconds (children).
    pub child_user_secs: f64,
    /// System CPU time in seconds (children).
    pub child_sys_secs: f64,
}

/// Get resource usage for the current process and its children.
///
/// On Unix: uses `getrusage(RUSAGE_SELF)` and `getrusage(RUSAGE_CHILDREN)`.
/// On Windows: returns zeros since the API is unavailable.
pub fn get_rusage() -> Result<ResourceUsage, ResourceError> {
    #[cfg(unix)]
    {
        get_rusage_unix()
    }
    #[cfg(windows)]
    {
        get_rusage_windows()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(ResourceError::Unsupported)
    }
}

#[cfg(unix)]
fn get_rusage_unix() -> Result<ResourceUsage, ResourceError> {
    // SAFETY: getrusage is always safe to call with valid pointers.
    unsafe {
        let mut self_usage = std::mem::zeroed::<libc::rusage>();
        let mut child_usage = std::mem::zeroed::<libc::rusage>();

        let ret_self = libc::getrusage(libc::RUSAGE_SELF, &mut self_usage);
        let ret_child = libc::getrusage(libc::RUSAGE_CHILDREN, &mut child_usage);

        if ret_self != 0 || ret_child != 0 {
            return Err(ResourceError::System(std::io::Error::last_os_error()));
        }

        let to_secs =
            |tv: libc::timeval| -> f64 { tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0) };

        Ok(ResourceUsage {
            user_secs: to_secs(self_usage.ru_utime),
            sys_secs: to_secs(self_usage.ru_stime),
            child_user_secs: to_secs(child_usage.ru_utime),
            child_sys_secs: to_secs(child_usage.ru_stime),
        })
    }
}

#[cfg(windows)]
fn get_rusage_windows() -> Result<ResourceUsage, ResourceError> {
    // Windows doesn't have getrusage. Return zeros as specified by POSIX
    // when the information is unavailable.
    Ok(ResourceUsage {
        user_secs: 0.0,
        sys_secs: 0.0,
        child_user_secs: 0.0,
        child_sys_secs: 0.0,
    })
}

/// Errors that can occur when querying resource usage.
#[derive(Debug)]
pub enum ResourceError {
    /// System call failed.
    System(std::io::Error),
    /// Platform not supported.
    Unsupported,
}

impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceError::System(e) => write!(f, "system error: {e}"),
            ResourceError::Unsupported => write!(f, "platform not supported"),
        }
    }
}

impl std::error::Error for ResourceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_rusage_does_not_panic() {
        // Should at least not panic on any platform.
        let _ = get_rusage();
    }

    #[test]
    #[cfg(windows)]
    fn windows_returns_zeros() {
        let usage = get_rusage().unwrap();
        assert_eq!(usage.user_secs, 0.0);
        assert_eq!(usage.sys_secs, 0.0);
        assert_eq!(usage.child_user_secs, 0.0);
        assert_eq!(usage.child_sys_secs, 0.0);
    }
}
