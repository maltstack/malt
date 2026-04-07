//! Overlayfs rootfs isolation primitive.
//!
//! Provides overlayfs mount/unmount for the Contained isolation tier.
//! Overlayfs layers a writable upper directory over a read-only lower
//! directory, with a work directory for internal filesystem operations.

use std::path::Path;

use super::tier::IsolationError;

/// Check whether overlayfs is supported on this system.
///
/// Probes by checking kernel module availability and mount support.
pub fn probe_overlayfs() -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        // Check that the overlay filesystem type is known
        let filesystems = std::fs::read_to_string("/proc/filesystems").map_err(|e| {
            IsolationError::OverlayfsError(format!("failed to read /proc/filesystems: {e}"))
        })?;

        if !filesystems.contains("overlay") {
            return Err(IsolationError::OverlayfsError(
                "overlay filesystem not found in /proc/filesystems".to_string(),
            ));
        }

        // Check for CAP_SYS_ADMIN
        if !nix::unistd::geteuid().is_root() {
            return Err(IsolationError::InsufficientCapabilities(
                "CAP_SYS_ADMIN required for overlayfs mount".to_string(),
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        Err(IsolationError::UnsupportedPlatform(
            "overlayfs is Linux-only".to_string(),
        ))
    }
}

/// Set up an overlayfs mount.
///
/// # Arguments
///
/// * `upper` — Writable upper directory (changes are stored here).
/// * `work` — Work directory (required by overlayfs, must be on same fs as upper).
/// * `lower` — Read-only lower directory (the base filesystem).
/// * `mount_point` — Where to mount the overlay.
///
/// # Layout
///
/// ```text
/// mount_point/          ← merged view (what the process sees)
/// ├── upper/            ← changes written by the process
/// ├── work/             ← internal overlayfs work area
/// └── lower/            ← read-only base (e.g., rootfs)
/// ```
pub fn setup_overlayfs(
    upper: &Path,
    work: &Path,
    lower: &Path,
    mount_point: &Path,
) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MsFlags;

        // Validate that directories exist
        for (name, path) in [("upper", upper), ("work", work), ("lower", lower)] {
            if !path.exists() {
                return Err(IsolationError::OverlayfsError(format!(
                    "{name} directory does not exist: {}",
                    path.display()
                )));
            }
            if !path.is_dir() {
                return Err(IsolationError::OverlayfsError(format!(
                    "{name} is not a directory: {}",
                    path.display()
                )));
            }
        }

        // Create mount point if it doesn't exist
        std::fs::create_dir_all(mount_point).map_err(|e| {
            IsolationError::OverlayfsError(format!(
                "failed to create mount point {}: {e}",
                mount_point.display()
            ))
        })?;

        // Build overlayfs options string
        let options = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower.display(),
            upper.display(),
            work.display()
        );

        // Mount the overlay filesystem
        nix::mount::mount(
            Some("overlay"),
            mount_point,
            Some("overlay"),
            MsFlags::empty(),
            Some(&options),
        )?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = upper;
        let _ = work;
        let _ = lower;
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "overlayfs is Linux-only".to_string(),
        ))
    }
}

/// Clean up an overlayfs mount by unmounting it.
pub fn cleanup_overlayfs(mount_point: &Path) -> Result<(), IsolationError> {
    #[cfg(target_os = "linux")]
    {
        use nix::mount::MntFlags;

        if !mount_point.exists() {
            return Ok(());
        }

        nix::mount::umount2(mount_point, MntFlags::empty()).map_err(|e| {
            IsolationError::OverlayfsError(format!(
                "failed to unmount overlay at {}: {e}",
                mount_point.display()
            ))
        })?;

        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(IsolationError::UnsupportedPlatform(
            "overlayfs is Linux-only".to_string(),
        ))
    }
}

/// Build a minimal rootfs layout for overlayfs.
///
/// Creates the directory structure needed for a container rootfs:
/// upper, work, lower, and mount_point directories.
///
/// The caller is responsible for populating the `lower` directory
/// with the base filesystem (e.g., from a rootfs tarball).
pub fn prepare_rootfs_dirs(
    base: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), IsolationError> {
    let upper = base.join("upper");
    let work = base.join("work");
    let lower = base.join("lower");
    let mount_point = base.join("root");

    for dir in [&upper, &work, &lower, &mount_point] {
        std::fs::create_dir_all(dir).map_err(|e| {
            IsolationError::OverlayfsError(format!(
                "failed to create directory {}: {e}",
                dir.display()
            ))
        })?;
    }

    Ok((upper, work, lower, mount_point))
}

use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn overlayfs_unavailable_on_non_linux() {
        assert!(probe_overlayfs().is_err());
        assert!(setup_overlayfs(
            Path::new("/tmp/upper"),
            Path::new("/tmp/work"),
            Path::new("/tmp/lower"),
            Path::new("/tmp/mount")
        )
        .is_err());
        assert!(cleanup_overlayfs(Path::new("/tmp/mount")).is_err());
    }

    #[test]
    fn prepare_rootfs_dirs_creates_structure() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let result = prepare_rootfs_dirs(base);
        assert!(result.is_ok());

        let (upper, work, lower, mount_point) = result.unwrap();
        assert!(upper.exists());
        assert!(work.exists());
        assert!(lower.exists());
        assert!(mount_point.exists());
        assert!(upper.is_dir());
        assert!(work.is_dir());
        assert!(lower.is_dir());
        assert!(mount_point.is_dir());
    }

    #[test]
    fn setup_overlayfs_validates_missing_dirs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base = temp_dir.path();

        let upper = base.join("upper");
        let work = base.join("work");
        let lower = base.join("lower");
        let mount_point = base.join("root");

        // None of the directories exist
        let result = setup_overlayfs(&upper, &work, &lower, &mount_point);

        #[cfg(target_os = "linux")]
        {
            assert!(result.is_err());
            if let Err(IsolationError::OverlayfsError(msg)) = result {
                assert!(msg.contains("does not exist"));
            } else {
                panic!("expected OverlayfsError");
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            assert!(result.is_err());
        }
    }

    #[test]
    fn cleanup_overlayfs_on_nonexistent_path_is_ok() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nonexistent = temp_dir.path().join("does_not_exist");

        let result = cleanup_overlayfs(&nonexistent);

        #[cfg(target_os = "linux")]
        {
            // On Linux, returns Ok because path doesn't exist (early return)
            assert!(result.is_ok());
        }

        #[cfg(not(target_os = "linux"))]
        {
            assert!(result.is_err());
        }
    }
}
