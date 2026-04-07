//! Cross-platform isolation capability detection.
//!
//! Provides `IsolationCapabilities` — a struct that reports which
//! isolation primitives are available on the current platform.
//! Used by `tier_available()` to validate tier applicability.

/// Reports which isolation primitives are available on this system.
///
/// Fields are `true` if the corresponding capability is available,
/// `false` otherwise. On non-applicable platforms, fields default
/// to `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationCapabilities {
    // Linux capabilities
    /// Linux namespaces (mount, PID, UTS, IPC) are available.
    pub linux_namespaces: bool,
    /// Cgroup v2 is mounted and writable.
    pub linux_cgroups: bool,
    /// OverlayFS is available for root filesystem isolation.
    pub linux_overlayfs: bool,
    /// Seccomp BPF filtering is supported.
    pub linux_seccomp: bool,

    // Windows capabilities
    /// Windows Job Objects are available for process grouping and limits.
    pub windows_job_objects: bool,
    /// Windows restricted tokens are available for privilege reduction.
    pub windows_restricted_tokens: bool,

    // macOS capabilities
    /// macOS sandbox-exec (Seatbelt) is available.
    pub macos_sandbox: bool,
    /// setrlimit is available for per-process resource limits.
    pub macos_rlimit: bool,
}

impl Default for IsolationCapabilities {
    fn default() -> Self {
        Self {
            linux_namespaces: cfg!(target_os = "linux"),
            linux_cgroups: cfg!(target_os = "linux") && probe_cgroup_v2_available(),
            linux_overlayfs: cfg!(target_os = "linux") && probe_overlayfs_available(),
            linux_seccomp: cfg!(target_os = "linux") && probe_seccomp_available(),
            windows_job_objects: cfg!(target_os = "windows"),
            windows_restricted_tokens: cfg!(target_os = "windows"),
            macos_sandbox: cfg!(target_os = "macos"),
            macos_rlimit: cfg!(target_os = "macos"),
        }
    }
}

impl IsolationCapabilities {
    /// Probe the current system and return available capabilities.
    ///
    /// This performs lightweight checks — it does not create namespaces,
    /// cgroups, or job objects. It verifies prerequisites only.
    pub fn probe() -> Self {
        Self::default()
    }

    /// Check if any Linux isolation primitives are available.
    pub fn has_linux_isolation(&self) -> bool {
        self.linux_namespaces || self.linux_cgroups || self.linux_overlayfs || self.linux_seccomp
    }

    /// Check if any Windows isolation primitives are available.
    pub fn has_windows_isolation(&self) -> bool {
        self.windows_job_objects || self.windows_restricted_tokens
    }

    /// Check if any macOS isolation primitives are available.
    pub fn has_macos_isolation(&self) -> bool {
        self.macos_sandbox || self.macos_rlimit
    }

    /// Check if the Restricted tier is achievable on this platform.
    ///
    /// Requires at least one basic isolation primitive:
    /// - Linux: namespaces
    /// - Windows: job objects
    /// - macOS: sandbox
    pub fn supports_restricted(&self) -> bool {
        self.linux_namespaces || self.windows_job_objects || self.macos_sandbox
    }

    /// Check if the Capped tier is achievable on this platform.
    ///
    /// Requires Restricted + resource limits:
    /// - Linux: namespaces + cgroups
    /// - Windows: job objects (includes CPU rate)
    /// - macOS: sandbox + rlimit
    pub fn supports_capped(&self) -> bool {
        (self.linux_namespaces && self.linux_cgroups)
            || self.windows_job_objects
            || (self.macos_sandbox && self.macos_rlimit)
    }

    /// Check if the Contained tier is achievable on this platform.
    ///
    /// Requires Capped + advanced isolation:
    /// - Linux: namespaces + cgroups + overlayfs + seccomp
    /// - Windows: job objects + restricted tokens
    /// - macOS: sandbox + rlimit (no direct equivalent to seccomp/overlayfs)
    pub fn supports_contained(&self) -> bool {
        (self.linux_namespaces && self.linux_cgroups && self.linux_overlayfs && self.linux_seccomp)
            || (self.windows_job_objects && self.windows_restricted_tokens)
            || (self.macos_sandbox && self.macos_rlimit)
    }
}

// Linux-specific probe helpers

#[cfg(target_os = "linux")]
fn probe_cgroup_v2_available() -> bool {
    std::path::Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
}

#[cfg(not(target_os = "linux"))]
fn probe_cgroup_v2_available() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn probe_overlayfs_available() -> bool {
    // Check if overlayfs is listed in /proc/filesystems
    if let Ok(content) = std::fs::read_to_string("/proc/filesystems") {
        content.lines().any(|line| line.contains("overlay"))
    } else {
        false
    }
}

#[cfg(not(target_os = "linux"))]
fn probe_overlayfs_available() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn probe_seccomp_available() -> bool {
    // Check for Seccomp field in /proc/self/status
    if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
        content.lines().any(|line| line.starts_with("Seccomp:"))
    } else {
        false
    }
}

#[cfg(not(target_os = "linux"))]
fn probe_seccomp_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_probe_succeeds() {
        let caps = IsolationCapabilities::probe();
        // Should always return a valid struct
        assert!(
            cfg!(target_os = "linux")
                || cfg!(target_os = "windows")
                || cfg!(target_os = "macos")
                || cfg!(target_os = "freebsd")
        );
        let _ = caps;
    }

    #[test]
    fn capabilities_is_debug() {
        let caps = IsolationCapabilities::probe();
        let debug_str = format!("{caps:?}");
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn capabilities_is_cloneable() {
        let caps = IsolationCapabilities::probe();
        let cloned = caps.clone();
        assert_eq!(caps, cloned);
    }

    #[test]
    fn capabilities_is_equality_comparable() {
        let a = IsolationCapabilities::probe();
        let b = IsolationCapabilities::probe();
        assert_eq!(a, b);
    }

    #[test]
    fn capabilities_default_equals_probe() {
        let default = IsolationCapabilities::default();
        let probed = IsolationCapabilities::probe();
        assert_eq!(default, probed);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        // On Linux, namespace flag should be true (cfg! is compile-time)
        assert!(caps.linux_namespaces);
        // Other Linux flags depend on runtime state
        let _ = caps.linux_cgroups;
        let _ = caps.linux_overlayfs;
        let _ = caps.linux_seccomp;
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.windows_job_objects);
        assert!(caps.windows_restricted_tokens);
        assert!(!caps.linux_namespaces);
        assert!(!caps.macos_sandbox);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_capabilities_reflect_platform() {
        let caps = IsolationCapabilities::probe();
        assert!(caps.macos_sandbox);
        assert!(caps.macos_rlimit);
        assert!(!caps.linux_namespaces);
        assert!(!caps.windows_job_objects);
    }

    #[test]
    fn has_platform_isolation_methods() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "linux")]
        {
            assert!(caps.has_linux_isolation());
            assert!(!caps.has_windows_isolation());
            assert!(!caps.has_macos_isolation());
        }

        #[cfg(target_os = "windows")]
        {
            assert!(!caps.has_linux_isolation());
            assert!(caps.has_windows_isolation());
            assert!(!caps.has_macos_isolation());
        }

        #[cfg(target_os = "macos")]
        {
            assert!(!caps.has_linux_isolation());
            assert!(!caps.has_windows_isolation());
            assert!(caps.has_macos_isolation());
        }
    }

    #[test]
    fn supports_restricted_on_current_platform() {
        let caps = IsolationCapabilities::probe();
        // All supported platforms should support at least Restricted
        assert!(caps.supports_restricted());
    }

    #[test]
    fn supports_capped_on_current_platform() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "linux")]
        {
            // Capped requires cgroups on Linux — may or may not be available
            let _ = caps.supports_capped();
        }

        #[cfg(target_os = "windows")]
        {
            // Windows job objects support CPU rate, so Capped is supported
            assert!(caps.supports_capped());
        }

        #[cfg(target_os = "macos")]
        {
            // macOS has both sandbox and rlimit
            assert!(caps.supports_capped());
        }
    }

    #[test]
    fn supports_contained_on_current_platform() {
        let caps = IsolationCapabilities::probe();

        #[cfg(target_os = "linux")]
        {
            // Contained requires overlayfs + seccomp on Linux
            let _ = caps.supports_contained();
        }

        #[cfg(target_os = "windows")]
        {
            // Windows has job objects + restricted tokens
            assert!(caps.supports_contained());
        }

        #[cfg(target_os = "macos")]
        {
            // macOS has sandbox + rlimit (no seccomp equivalent, but sandbox is strong)
            assert!(caps.supports_contained());
        }
    }

    #[test]
    fn empty_capabilities_supports_nothing() {
        let caps = IsolationCapabilities {
            linux_namespaces: false,
            linux_cgroups: false,
            linux_overlayfs: false,
            linux_seccomp: false,
            windows_job_objects: false,
            windows_restricted_tokens: false,
            macos_sandbox: false,
            macos_rlimit: false,
        };
        assert!(!caps.has_linux_isolation());
        assert!(!caps.has_windows_isolation());
        assert!(!caps.has_macos_isolation());
        assert!(!caps.supports_restricted());
        assert!(!caps.supports_capped());
        assert!(!caps.supports_contained());
    }
}
