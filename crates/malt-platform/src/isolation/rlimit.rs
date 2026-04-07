//! macOS setrlimit for memory and CPU limits.
//!
//! Provides resource limit enforcement via `setrlimit(2)`.
//! Unlike cgroups on Linux, rlimits are per-process and cannot
//! be applied to a group of processes atomically.

use super::tier::IsolationError;

/// Resource limit configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Maximum resident set size in bytes (RLIMIT_RSS).
    pub memory_limit: Option<u64>,
    /// Maximum CPU time in seconds (RLIMIT_CPU).
    pub cpu_limit: Option<u64>,
    /// Maximum file size in bytes (RLIMIT_FSIZE).
    pub file_size_limit: Option<u64>,
    /// Maximum number of open file descriptors (RLIMIT_NOFILE).
    pub nofile_limit: Option<u64>,
    /// Maximum number of processes (RLIMIT_NPROC).
    pub nproc_limit: Option<u64>,
    /// Maximum core dump size in bytes (RLIMIT_CORE).
    pub core_limit: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_limit: None,
            cpu_limit: None,
            file_size_limit: None,
            nofile_limit: None,
            nproc_limit: None,
            core_limit: None,
        }
    }
}

impl ResourceLimits {
    /// Create a new ResourceLimits with all limits unset.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the memory limit in bytes.
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// Set the CPU time limit in seconds.
    pub fn with_cpu_limit(mut self, seconds: u64) -> Self {
        self.cpu_limit = Some(seconds);
        self
    }

    /// Set the maximum file size in bytes.
    pub fn with_file_size_limit(mut self, bytes: u64) -> Self {
        self.file_size_limit = Some(bytes);
        self
    }

    /// Set the maximum number of open file descriptors.
    pub fn with_nofile_limit(mut self, limit: u64) -> Self {
        self.nofile_limit = Some(limit);
        self
    }

    /// Set the maximum number of processes.
    pub fn with_nproc_limit(mut self, limit: u64) -> Self {
        self.nproc_limit = Some(limit);
        self
    }

    /// Set the maximum core dump size in bytes.
    /// Set to 0 to disable core dumps.
    pub fn with_core_limit(mut self, bytes: u64) -> Self {
        self.core_limit = Some(bytes);
        self
    }

    /// Apply all configured resource limits to the current process.
    ///
    /// Limits are applied in order: memory, CPU, file size, nofile, nproc, core.
    /// If any limit fails, previously applied limits remain in effect.
    pub fn apply(&self) -> Result<(), IsolationError> {
        if let Some(limit) = self.memory_limit {
            set_memory_limit(limit)?;
        }
        if let Some(limit) = self.cpu_limit {
            set_cpu_limit(limit)?;
        }
        if let Some(limit) = self.file_size_limit {
            set_file_size_limit(limit)?;
        }
        if let Some(limit) = self.nofile_limit {
            set_nofile_limit(limit)?;
        }
        if let Some(limit) = self.nproc_limit {
            set_nproc_limit(limit)?;
        }
        if let Some(limit) = self.core_limit {
            set_core_limit(limit)?;
        }
        Ok(())
    }

    /// Check if any limits are configured.
    pub fn has_limits(&self) -> bool {
        self.memory_limit.is_some()
            || self.cpu_limit.is_some()
            || self.file_size_limit.is_some()
            || self.nofile_limit.is_some()
            || self.nproc_limit.is_some()
            || self.core_limit.is_some()
    }
}

/// Set the memory limit (RLIMIT_RSS) for the current process.
///
/// # Arguments
///
/// * `bytes` — Maximum resident set size in bytes.
///
/// # Note
///
/// On modern macOS, RLIMIT_RSS may not be enforced by the kernel.
/// It serves as a hint to the system and may be ignored. For hard
/// memory limits, consider using launchd's `MemoryLimit` key or
/// combining with the sandbox profile.
pub fn set_memory_limit(bytes: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: bytes as libc::rlim_t,
            rlim_max: bytes as libc::rlim_t,
        };

        // SAFETY: rlim is a valid rlimit struct. RLIMIT_RSS is a valid
        // resource constant on macOS. setrlimit is safe to call.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_RSS, &rlim) };

        if result != 0 {
            // SAFETY: errno is thread-local and set by setrlimit on failure.
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_RSS, {bytes}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Set the CPU time limit (RLIMIT_CPU) for the current process.
///
/// # Arguments
///
/// * `seconds` — Maximum CPU time in seconds. When exceeded, the process
///   receives `SIGXCPU`. If the limit is not raised, the process is
///   terminated with `SIGKILL`.
pub fn set_cpu_limit(seconds: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: seconds as libc::rlim_t,
            rlim_max: seconds as libc::rlim_t,
        };

        // SAFETY: rlim is a valid rlimit struct. RLIMIT_CPU is a valid
        // resource constant on all Unix systems.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_CPU, &rlim) };

        if result != 0 {
            // SAFETY: errno is thread-local and set by setrlimit on failure.
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_CPU, {seconds}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = seconds;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Set the maximum file size (RLIMIT_FSIZE) for the current process.
///
/// Exceeding this limit causes writes to fail with `EFBIG` and
/// the process receives `SIGXFSZ`.
pub fn set_file_size_limit(bytes: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: bytes as libc::rlim_t,
            rlim_max: bytes as libc::rlim_t,
        };

        // SAFETY: rlim is valid, RLIMIT_FSIZE is valid on macOS.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_FSIZE, {bytes}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Set the maximum number of open file descriptors (RLIMIT_NOFILE).
pub fn set_nofile_limit(limit: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: limit as libc::rlim_t,
            rlim_max: limit as libc::rlim_t,
        };

        // SAFETY: rlim is valid, RLIMIT_NOFILE is valid on macOS.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_NOFILE, {limit}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = limit;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Set the maximum number of processes (RLIMIT_NPROC).
pub fn set_nproc_limit(limit: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: limit as libc::rlim_t,
            rlim_max: limit as libc::rlim_t,
        };

        // SAFETY: rlim is valid, RLIMIT_NPROC is valid on macOS.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_NPROC, &rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_NPROC, {limit}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = limit;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Set the maximum core dump size (RLIMIT_CORE).
/// Set to 0 to disable core dumps entirely.
pub fn set_core_limit(bytes: u64) -> Result<(), IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let rlim = libc::rlimit {
            rlim_cur: bytes as libc::rlim_t,
            rlim_max: bytes as libc::rlim_t,
        };

        // SAFETY: rlim is valid, RLIMIT_CORE is valid on macOS.
        let result = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "setrlimit(RLIMIT_CORE, {bytes}) failed: errno={errno}"
            )));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        Err(IsolationError::UnsupportedPlatform(
            "setrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Get the current memory limit (RLIMIT_RSS).
///
/// Returns `None` if the limit is unlimited (`RLIM_INFINITY`).
pub fn get_memory_limit() -> Result<Option<u64>, IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        // SAFETY: rlim is a valid mutable pointer. RLIMIT_RSS is valid.
        let result = unsafe { libc::getrlimit(libc::RLIMIT_RSS, &mut rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "getrlimit(RLIMIT_RSS) failed: errno={errno}"
            )));
        }

        if rlim.rlim_cur == libc::RLIM_INFINITY as libc::rlim_t {
            Ok(None)
        } else {
            Ok(Some(rlim.rlim_cur as u64))
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "getrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

/// Get the current CPU time limit (RLIMIT_CPU).
///
/// Returns `None` if the limit is unlimited.
pub fn get_cpu_limit() -> Result<Option<u64>, IsolationError> {
    #[cfg(target_os = "macos")]
    {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };

        // SAFETY: rlim is valid, RLIMIT_CPU is valid.
        let result = unsafe { libc::getrlimit(libc::RLIMIT_CPU, &mut rlim) };

        if result != 0 {
            let errno = unsafe { *libc::__error() };
            return Err(IsolationError::RlimitError(format!(
                "getrlimit(RLIMIT_CPU) failed: errno={errno}"
            )));
        }

        if rlim.rlim_cur == libc::RLIM_INFINITY as libc::rlim_t {
            Ok(None)
        } else {
            Ok(Some(rlim.rlim_cur as u64))
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "getrlimit is Unix-only (macOS implementation)".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_limits_default_has_no_limits() {
        let limits = ResourceLimits::default();
        assert!(!limits.has_limits());
    }

    #[test]
    fn resource_limits_new_has_no_limits() {
        let limits = ResourceLimits::new();
        assert!(!limits.has_limits());
    }

    #[test]
    fn resource_limits_builder_pattern() {
        let limits = ResourceLimits::new()
            .with_memory_limit(256 * 1024 * 1024)
            .with_cpu_limit(60);
        assert!(limits.has_limits());
        assert_eq!(limits.memory_limit, Some(256 * 1024 * 1024));
        assert_eq!(limits.cpu_limit, Some(60));
    }

    #[test]
    fn resource_limits_all_fields() {
        let limits = ResourceLimits::new()
            .with_memory_limit(512 * 1024 * 1024)
            .with_cpu_limit(120)
            .with_file_size_limit(1024 * 1024 * 1024)
            .with_nofile_limit(256)
            .with_nproc_limit(50)
            .with_core_limit(0);
        assert!(limits.has_limits());
        assert_eq!(limits.memory_limit, Some(512 * 1024 * 1024));
        assert_eq!(limits.cpu_limit, Some(120));
        assert_eq!(limits.file_size_limit, Some(1024 * 1024 * 1024));
        assert_eq!(limits.nofile_limit, Some(256));
        assert_eq!(limits.nproc_limit, Some(50));
        assert_eq!(limits.core_limit, Some(0));
    }

    #[test]
    fn resource_limits_is_debug() {
        let limits = ResourceLimits::new().with_memory_limit(1024);
        let debug_str = format!("{limits:?}");
        assert!(debug_str.contains("memory_limit"));
    }

    #[test]
    fn resource_limits_is_cloneable() {
        let limits = ResourceLimits::new().with_cpu_limit(30);
        let cloned = limits.clone();
        assert_eq!(limits.cpu_limit, cloned.cpu_limit);
    }

    #[test]
    fn resource_limits_is_copyable() {
        let limits = ResourceLimits::new().with_nofile_limit(128);
        let copied = limits;
        // If ResourceLimits is Copy, both variables should be usable
        assert_eq!(copied.nofile_limit, Some(128));
        assert_eq!(limits.nofile_limit, Some(128));
    }

    #[test]
    fn resource_limits_is_equality_comparable() {
        let a = ResourceLimits::new().with_memory_limit(100);
        let b = ResourceLimits::new().with_memory_limit(100);
        let c = ResourceLimits::new().with_memory_limit(200);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn rlimit_functions_unavailable_on_non_macos() {
        assert!(set_memory_limit(1024).is_err());
        assert!(set_cpu_limit(60).is_err());
        assert!(set_file_size_limit(1024).is_err());
        assert!(set_nofile_limit(256).is_err());
        assert!(set_nproc_limit(50).is_err());
        assert!(set_core_limit(0).is_err());
        assert!(get_memory_limit().is_err());
        assert!(get_cpu_limit().is_err());
    }

    #[test]
    fn rlimit_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IsolationError>();
    }

    #[test]
    fn rlimit_error_is_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<IsolationError>();
    }
}
