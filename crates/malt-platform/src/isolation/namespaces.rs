//! Linux namespace isolation primitives.
//!
//! Provides mount, PID, UTS, and IPC namespace creation via `unshare(2)`.
//! Network namespace handling is in the `network` module.

use super::tier::{IsolationError, IsolationTier};

/// Linux namespace flags used for unshare.
#[cfg(target_os = "linux")]
use nix::sched::CloneFlags;

/// Check whether namespace isolation is supported on this system.
///
/// Probes by reading `/proc/self/ns/` entries — if the kernel exposes
/// namespace identifiers, unshare is likely supported.
pub fn probe_namespaces() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        // Check that /proc/self/ns exists (namespace support indicator)
        if !std::path::Path::new("/proc/self/ns").exists() {
            return Err(IsolationError::NamespaceError(
                "/proc/self/ns not found — namespace support unavailable".to_string(),
            ));
        }

        // Check for CAP_SYS_ADMIN
        if !nix::unistd::geteuid().is_root() {
            return Err(IsolationError::InsufficientCapabilities(
                "CAP_SYS_ADMIN required for namespace creation".to_string(),
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "namespaces are Linux-only".to_string(),
        ))
    }
}

/// Create a new PID namespace for the calling process.
///
/// After unshare, the calling process becomes PID 1 in the new namespace.
/// Children spawned after this call will be in the new PID namespace.
///
/// **Note:** The calling process does NOT become PID 1 in its own view —
/// only children see the new namespace. Use `clone()` with `CLONE_NEWPID`
/// for the parent to see itself as PID 1.
pub fn create_pid_namespace() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        nix::sched::unshare(CloneFlags::CLONE_NEWPID)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "PID namespaces are Linux-only".to_string(),
        ))
    }
}

/// Create a new mount namespace for the calling process.
///
/// The calling process gets a private copy of its mount table.
/// Subsequent mount/unmount operations do not affect the parent namespace.
pub fn create_mount_namespace() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        nix::sched::unshare(CloneFlags::CLONE_NEWNS)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "mount namespaces are Linux-only".to_string(),
        ))
    }
}

/// Create a new UTS namespace for the calling process.
///
/// Allows setting a hostname independent of the host system.
pub fn create_uts_namespace() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        nix::sched::unshare(CloneFlags::CLONE_NEWUTS)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "UTS namespaces are Linux-only".to_string(),
        ))
    }
}

/// Create a new IPC namespace for the calling process.
///
/// Isolates System V IPC objects and POSIX message queues.
pub fn create_ipc_namespace() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        nix::sched::unshare(CloneFlags::CLONE_NEWIPC)?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "IPC namespaces are Linux-only".to_string(),
        ))
    }
}

/// Apply all namespace isolations appropriate for the given tier.
///
/// | Tier         | Namespaces applied                          |
/// |--------------|---------------------------------------------|
/// | Bare         | None                                        |
/// | Restricted   | PID, mount, UTS, IPC + NO_NEW_PRIVS         |
/// | Capped       | Same as Restricted                          |
/// | Contained    | Same as Restricted (+ network in net module) |
pub fn unshare_namespaces(tier: IsolationTier) -> Result<(), IsolationError> {
    match tier {
        IsolationTier::Bare => Ok(()),
        IsolationTier::Restricted | IsolationTier::Capped | IsolationTier::Contained => {
            // Order matters: mount first so subsequent namespace setups
            // can mount private filesystems without affecting the host.
            create_mount_namespace()?;
            create_pid_namespace()?;
            create_uts_namespace()?;
            create_ipc_namespace()?;

            // Apply PR_SET_NO_NEW_PRIVS to prevent privilege escalation
            // via setuid binaries or file capabilities.
            #[cfg(target_os = "linux")]
            {
                nix::sys::prctl::set_no_new_privs()?;
            }

            Ok(())
        }
    }
}

/// Mount a tmpfs at the given path (useful for private /tmp in namespaces).
pub fn mount_tmpfs(mount_point: &std::path::Path) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MsFlags;

        std::fs::create_dir_all(mount_point)?;

        nix::mount::mount(
            Some("tmpfs"),
            mount_point,
            Some("tmpfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            Some("size=64M"),
        )?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "tmpfs mount is Linux-only".to_string(),
        ))
    }
}

/// Mount procfs at the given path (typically `/proc` in a new namespace).
pub fn mount_proc(mount_point: &std::path::Path) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MsFlags;

        std::fs::create_dir_all(mount_point)?;

        nix::mount::mount(
            Some("proc"),
            mount_point,
            Some("proc"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
            None::<&str>,
        )?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "proc mount is Linux-only".to_string(),
        ))
    }
}

/// Mount sysfs at the given path (typically `/sys` in a new namespace).
pub fn mount_sysfs(mount_point: &std::path::Path) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MsFlags;

        std::fs::create_dir_all(mount_point)?;

        nix::mount::mount(
            Some("sysfs"),
            mount_point,
            Some("sysfs"),
            MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_RDONLY,
            None::<&str>,
        )?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "sysfs mount is Linux-only".to_string(),
        ))
    }
}

/// Unmount a filesystem at the given path.
pub fn unmount(mount_point: &std::path::Path) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MntFlags;

        nix::mount::umount2(mount_point, MntFlags::empty())?;
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "umount is Linux-only".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unshare_bare_is_noop() {
        assert!(unshare_namespaces(IsolationTier::Bare).is_ok());
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn namespace_functions_unavailable_on_non_linux() {
        assert!(create_pid_namespace().is_err());
        assert!(create_mount_namespace().is_err());
        assert!(create_uts_namespace().is_err());
        assert!(create_ipc_namespace().is_err());
        assert!(mount_tmpfs(std::path::Path::new("/tmp/test")).is_err());
        assert!(mount_proc(std::path::Path::new("/proc/test")).is_err());
        assert!(mount_sysfs(std::path::Path::new("/sys/test")).is_err());
        assert!(unmount(std::path::Path::new("/tmp/test")).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn probe_requires_root() {
        // This test will pass if running as root, fail otherwise.
        // In CI, this should be run both ways.
        let result = probe_namespaces();
        if !nix::unistd::geteuid().is_root() {
            assert!(result.is_err());
        }
    }

    #[test]
    fn unshare_non_bare_requires_linux() {
        let result = unshare_namespaces(IsolationTier::Restricted);
        #[cfg(not(target_os = "linux"))]
        {
            assert!(result.is_err());
        }
        #[cfg(target_os = "linux")]
        {
            // On Linux, this may fail due to lack of root — that's expected.
            // We only verify it doesn't panic.
            let _ = result;
        }
    }

    #[test]
    fn isolation_error_from_nix_eperm() {
        #[cfg(target_os = "linux")]
        {
            let nix_err = nix::Error::EPERM;
            let err = IsolationError::from(nix_err);
            assert!(matches!(err, IsolationError::InsufficientCapabilities(_)));
        }
    }

    #[test]
    fn isolation_error_from_nix_eacces() {
        #[cfg(target_os = "linux")]
        {
            let nix_err = nix::Error::EACCES;
            let err = IsolationError::from(nix_err);
            assert!(matches!(err, IsolationError::InsufficientCapabilities(_)));
        }
    }

    #[test]
    fn isolation_error_from_nix_other() {
        #[cfg(target_os = "linux")]
        {
            let nix_err = nix::Error::EINVAL;
            let err = IsolationError::from(nix_err);
            assert!(matches!(err, IsolationError::NamespaceError(_)));
        }
    }
}
