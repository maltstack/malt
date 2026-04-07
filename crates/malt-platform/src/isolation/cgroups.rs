//! Cgroup v2 memory and CPU limit primitives.
//!
//! Provides cgroup v2 management for memory and CPU quotas.
//! Cgroup v2 uses a unified hierarchy at `/sys/fs/cgroup/`.

use std::path::{Path, PathBuf};

use super::tier::IsolationError;

/// Represents a cgroup v2 group with its path.
#[derive(Debug, Clone)]
pub struct Cgroup {
    /// The cgroup name (relative to the cgroup mount point).
    name: String,
    /// The absolute path to the cgroup directory.
    path: PathBuf,
}

impl Cgroup {
    /// Get the cgroup name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the absolute path to the cgroup directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The cgroup v2 mount point.
const CGROUP_V2_MOUNT: &str = "/sys/fs/cgroup";

/// Check whether cgroup v2 is available on this system.
///
/// Probes by checking for the cgroup v2 mount point and verifying
/// that the unified hierarchy is in use.
pub fn probe_cgroup_v2() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        let cgroup_path = Path::new(CGROUP_V2_MOUNT);

        if !cgroup_path.exists() {
            return Err(IsolationError::CgroupError(
                format!("{CGROUP_V2_MOUNT} not found — cgroup v2 not mounted").to_string(),
            ));
        }

        // Check that cgroup2 filesystem type is mounted
        // by reading /proc/filesystems or checking cgroup.controllers
        let controllers_file = cgroup_path.join("cgroup.controllers");
        if !controllers_file.exists() {
            return Err(IsolationError::CgroupError(
                "cgroup.controllers not found — cgroup v2 not available".to_string(),
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "cgroups are Linux-only".to_string(),
        ))
    }
}

/// Create a new cgroup v2 with memory and CPU limits.
///
/// # Arguments
///
/// * `name` — The cgroup name (will be created under `/sys/fs/cgroup/`).
/// * `memory_limit_mb` — Memory limit in megabytes (0 = no limit).
/// * `cpu_quota_pct` — CPU quota as a percentage of one CPU (100 = 1 full CPU, 0 = no limit).
///
/// # Example
///
/// ```ignore
/// let cgroup = create_cgroup("malt-session-42", 512, 200)?;
/// // Creates /sys/fs/cgroup/malt-session-42 with 512MB memory limit
/// // and 200% CPU quota (2 CPUs worth).
/// ```
pub fn create_cgroup(
    name: &str,
    memory_limit_mb: u64,
    cpu_quota_pct: u64,
) -> Result<Cgroup, IsolationError> {
    #[cfg(target_os = "linux")]
    {
        let cgroup_path = Path::new(CGROUP_V2_MOUNT).join(name);

        // Create the cgroup directory
        std::fs::create_dir_all(&cgroup_path).map_err(|e| {
            IsolationError::CgroupError(format!(
                "failed to create cgroup directory {}: {e}",
                cgroup_path.display()
            ))
        })?;

        // Set memory limit if specified
        if memory_limit_mb > 0 {
            let memory_bytes = memory_limit_mb * 1024 * 1024;
            let memory_max_path = cgroup_path.join("memory.max");
            std::fs::write(&memory_max_path, memory_bytes.to_string()).map_err(|e| {
                IsolationError::CgroupError(format!(
                    "failed to set memory.max at {}: {e}",
                    memory_max_path.display()
                ))
            })?;
        }

        // Set CPU quota if specified
        if cpu_quota_pct > 0 {
            // cpu.max format: "quota period" (in microseconds)
            // For percentage: quota = (pct / 100) * period
            // We use a period of 100000us (100ms) for granularity
            let period: u64 = 100_000;
            let quota = (cpu_quota_pct * period) / 100;
            let cpu_max_path = cgroup_path.join("cpu.max");
            let cpu_max_value = format!("{quota} {period}");
            std::fs::write(&cpu_max_path, &cpu_max_value).map_err(|e| {
                IsolationError::CgroupError(format!(
                    "failed to set cpu.max at {}: {e}",
                    cpu_max_path.display()
                ))
            })?;
        }

        Ok(Cgroup {
            name: name.to_string(),
            path: cgroup_path,
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = name;
        let _ = memory_limit_mb;
        let _ = cpu_quota_pct;
        Err(IsolationError::UnsupportedPlatform(
            "cgroups are Linux-only".to_string(),
        ))
    }
}

/// Add a process to a cgroup by writing its PID to `cgroup.procs`.
pub fn add_process_to_cgroup(cgroup: &Cgroup, pid: u32) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        let procs_path = cgroup.path.join("cgroup.procs");
        std::fs::write(&procs_path, pid.to_string()).map_err(|e| {
            IsolationError::CgroupError(format!(
                "failed to add PID {pid} to cgroup at {}: {e}",
                procs_path.display()
            ))
        })?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = cgroup;
        let _ = pid;
        Err(IsolationError::UnsupportedPlatform(
            "cgroups are Linux-only".to_string(),
        ))
    }
}

/// Remove a cgroup by removing its directory.
///
/// The cgroup must be empty (no processes) before removal.
pub fn remove_cgroup(cgroup: &Cgroup) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        // Reset limits before removal for cleanliness
        let memory_max_path = cgroup.path.join("memory.max");
        if memory_max_path.exists() {
            let _ = std::fs::write(&memory_max_path, "max");
        }

        let cpu_max_path = cgroup.path.join("cpu.max");
        if cpu_max_path.exists() {
            let _ = std::fs::write(&cpu_max_path, "max 100000");
        }

        std::fs::remove_dir(&cgroup.path).map_err(|e| {
            IsolationError::CgroupError(format!(
                "failed to remove cgroup directory {}: {e}",
                cgroup.path.display()
            ))
        })?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = cgroup;
        Err(IsolationError::UnsupportedPlatform(
            "cgroups are Linux-only".to_string(),
        ))
    }
}

/// Get the current memory usage of a cgroup in bytes.
pub fn get_memory_usage(cgroup: &Cgroup) -> Result<u64, IsolationError> {
    #[cfg(target_os = "linux")]
    {
        let current_path = cgroup.path.join("memory.current");
        let content = std::fs::read_to_string(&current_path).map_err(|e| {
            IsolationError::CgroupError(format!(
                "failed to read memory.current at {}: {e}",
                current_path.display()
            ))
        })?;

        // Parse "current <bytes>" from the file
        for line in content.lines() {
            if line.starts_with("current ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse().map_err(|e| {
                        IsolationError::CgroupError(format!(
                            "failed to parse memory usage '{:?}': {e}",
                            parts[1]
                        ))
                    });
                }
            }
        }

        Err(IsolationError::CgroupError(
            "could not find 'current' line in memory.current".to_string(),
        ))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = cgroup;
        Err(IsolationError::UnsupportedPlatform(
            "cgroups are Linux-only".to_string(),
        ))
    }
}

/// Get the cgroup v2 mount point path.
pub fn cgroup_mount_point() -> &'static Path {
    Path::new(CGROUP_V2_MOUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_name_accessor() {
        let cgroup = Cgroup {
            name: "test-group".to_string(),
            path: PathBuf::from("/sys/fs/cgroup/test-group"),
        };
        assert_eq!(cgroup.name(), "test-group");
    }

    #[test]
    fn cgroup_path_accessor() {
        let cgroup = Cgroup {
            name: "test-group".to_string(),
            path: PathBuf::from("/sys/fs/cgroup/test-group"),
        };
        assert_eq!(cgroup.path(), Path::new("/sys/fs/cgroup/test-group"));
    }

    #[test]
    fn cgroup_is_cloneable() {
        let cgroup = Cgroup {
            name: "test".to_string(),
            path: PathBuf::from("/sys/fs/cgroup/test"),
        };
        let cloned = cgroup.clone();
        assert_eq!(cgroup.name(), cloned.name());
        assert_eq!(cgroup.path(), cloned.path());
    }

    #[test]
    fn cgroup_is_debug() {
        let cgroup = Cgroup {
            name: "test".to_string(),
            path: PathBuf::from("/sys/fs/cgroup/test"),
        };
        let debug_str = format!("{cgroup:?}");
        assert!(debug_str.contains("test"));
    }

    #[test]
    fn cgroup_mount_point_returns_path() {
        let path = cgroup_mount_point();
        assert_eq!(path, Path::new(CGROUP_V2_MOUNT));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn cgroup_functions_unavailable_on_non_linux() {
        assert!(create_cgroup("test", 256, 100).is_err());
        let cgroup = Cgroup {
            name: "test".to_string(),
            path: PathBuf::from("/sys/fs/cgroup/test"),
        };
        assert!(add_process_to_cgroup(&cgroup, 1).is_err());
        assert!(remove_cgroup(&cgroup).is_err());
        assert!(get_memory_usage(&cgroup).is_err());
        assert!(probe_cgroup_v2().is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn probe_requires_cgroup_mount() {
        let result = probe_cgroup_v2();
        if !Path::new(CGROUP_V2_MOUNT).exists() {
            assert!(result.is_err());
        }
    }
}
