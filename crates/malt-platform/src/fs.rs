//! Cross-platform filesystem utilities.
//!
//! This module provides path translation and virtual device handling
//! for POSIX compatibility on Windows.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Map POSIX device file paths to their platform equivalents.
///
/// On Windows, translates `/dev/null` → `NUL`, `/dev/stdin` → `CON`, etc.
/// On Unix, returns the path unchanged.
pub fn normalize_device_path(path: &str) -> Cow<'_, str> {
    #[cfg(windows)]
    {
        match path {
            "/dev/null" => Cow::Borrowed("NUL"),
            "/dev/zero" => Cow::Borrowed("NUL"), // Will be handled specially
            "/dev/urandom" | "/dev/random" => Cow::Borrowed("NUL"), // Will be handled specially
            "/dev/full" => Cow::Borrowed("NUL"), // Will be handled specially
            "/dev/stdin" => Cow::Borrowed("CON"),
            "/dev/stdout" => Cow::Borrowed("CON"),
            "/dev/stderr" => Cow::Borrowed("CON"),
            "/dev/tty" => Cow::Borrowed("CON"),
            _ => Cow::Borrowed(path),
        }
    }
    #[cfg(not(windows))]
    {
        Cow::Borrowed(path)
    }
}

/// Check if a path is a POSIX device path that needs special handling.
pub fn classify_virtual_dev(path: &str) -> Option<VirtualDevKind> {
    match path {
        "/dev/null" => Some(VirtualDevKind::Null),
        "/dev/zero" => Some(VirtualDevKind::Zero),
        "/dev/urandom" | "/dev/random" => Some(VirtualDevKind::Urandom),
        "/dev/full" => Some(VirtualDevKind::Full),
        "/dev/tty" => Some(VirtualDevKind::Tty),
        "/dev/stdin" | "/dev/stdout" | "/dev/stderr" => Some(VirtualDevKind::StdStream),
        p if p.starts_with("/dev/fd/") => Some(VirtualDevKind::Fd),
        _ => None,
    }
}

/// Virtual device types that need special handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDevKind {
    Null,
    Zero,
    Urandom,
    Full,
    Tty,
    StdStream,
    Fd,
}

/// Convert a POSIX-style path to a Windows native path.
///
/// Examples:
/// - `/c/Users/foo` → `C:\Users\foo`
/// - `/mnt/c/Users/foo` → `C:\Users\foo` (WSL2 mounts)
/// - `/tmp` → `%LOCALAPPDATA%\Temp\malt\{session}\`
/// - `/tmp/foo` → `%LOCALAPPDATA%\Temp\malt\{session}\foo`
pub fn to_windows_path(posix: &Path) -> PathBuf {
    let s = posix.to_string_lossy();

    // Already Windows path? Return as-is.
    if s.contains(":\\") || s.starts_with("\\\\") {
        return posix.to_path_buf();
    }

    // /tmp mapping: /tmp → session temp dir
    if s == "/tmp" {
        return malt_tmp_dir();
    }
    if let Some(rest) = s.strip_prefix("/tmp/") {
        return malt_tmp_dir().join(rest);
    }

    // WSL2 mount: /mnt/X/... → X:\...
    if let Some(rest) = s.strip_prefix("/mnt/").or_else(|| s.strip_prefix("/MNT/")) {
        if let Some(pb) = try_drive_prefix(rest) {
            return pb;
        }
    }

    // Cygwin/Git Bash: /X/... → X:\...
    if let Some(inner) = s.strip_prefix('/') {
        if let Some(pb) = try_drive_prefix(inner) {
            return pb;
        }
    }

    posix.to_path_buf()
}

/// Extract drive letter from POSIX path prefix.
/// Returns None if not a drive prefix.
fn try_drive_prefix(path: &str) -> Option<PathBuf> {
    let mut chars = path.chars();
    let drive = chars.next()?;

    if !drive.is_ascii_alphabetic() {
        return None;
    }

    let rest: String = chars.collect();

    // Case 1: Just drive letter (e.g., "c" from /c) -> C:\
    if rest.is_empty() {
        return Some(PathBuf::from(format!("{}:\\", drive.to_ascii_uppercase())));
    }

    // Case 2: Drive + separator + path (e.g., "c/Users/foo")
    let separator = rest.chars().next()?;
    if separator != '/' && separator != '\\' {
        return None;
    }

    let after_sep: String = rest.chars().skip(1).collect();
    Some(PathBuf::from(format!(
        "{}:\\{}",
        drive.to_ascii_uppercase(),
        after_sep.replace('/', "\\")
    )))
}

/// Return the MALT per-session temporary directory.
///
/// - **Unix:** `/tmp` (no mapping needed)
/// - **Windows:**
///   - If `MALT_SESSION_ID` is set (daemon mode):
///     `%LOCALAPPDATA%\Temp\malt\{session_id}\`
///   - Otherwise (standalone shell):
///     `%LOCALAPPDATA%\Temp\malt\standalone-{pid}\`
///
/// Creates the directory if it doesn't exist.
pub fn malt_tmp_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
    }

    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());

        let session = std::env::var("MALT_SESSION_ID")
            .unwrap_or_else(|_| format!("standalone-{}", std::process::id()));

        let dir = base.join("Temp").join("malt").join(session);

        // Create if doesn't exist
        let _ = std::fs::create_dir_all(&dir);

        dir
    }
}

#[cfg(windows)]
fn permission_db_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Temp"))
        .or_else(|_| std::env::current_dir().map(|p| p.join(".malt-permdb")))
        .unwrap_or_else(|_| PathBuf::from(".malt-permdb"));
    let dir = base.join("malt").join("permdb");
    let _ = fs::create_dir_all(&dir);
    dir
}

#[cfg(windows)]
fn permission_record_path(path: &Path) -> PathBuf {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    let hash = fnv1a64(canonical.as_bytes());
    permission_db_dir().join(format!("{hash:016x}.mode"))
}

#[cfg(windows)]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(windows)]
fn default_windows_mode(path: &Path) -> io::Result<u32> {
    let metadata = fs::metadata(path)?;
    let writable = !metadata.permissions().readonly();
    let mut mode = if metadata.is_dir() { 0o555 } else { 0o444 };
    if writable {
        mode |= 0o222;
    }
    if metadata.is_dir() {
        mode |= 0o111;
    }
    Ok(mode)
}

#[cfg(windows)]
fn read_mode_override(path: &Path) -> Option<u32> {
    let record = permission_record_path(path);
    let contents = fs::read_to_string(record).ok()?;
    u32::from_str_radix(contents.trim(), 8).ok()
}

#[cfg(windows)]
fn write_mode_override(path: &Path, mode: u32) -> io::Result<()> {
    let record = permission_record_path(path);
    fs::write(record, format!("{mode:o}\n"))
}

/// Set a POSIX-style mode for a path.
///
/// On Unix this forwards to the OS permission bits.
/// On Windows this persists a shell-visible mode overlay used by MALT tools
/// and shell builtins to emulate POSIX readability/writability checks.
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)
    }

    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(mode & 0o222 == 0);
        fs::set_permissions(path, permissions)?;
        write_mode_override(path, mode)
    }
}

/// Return the effective POSIX-style mode for a path.
pub fn get_mode(path: &Path) -> io::Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o777)
    }

    #[cfg(windows)]
    {
        if let Some(mode) = read_mode_override(path) {
            return Ok(mode);
        }
        default_windows_mode(path)
    }
}

/// Return whether the path should be treated as readable by the shell.
pub fn is_readable(path: &Path) -> bool {
    get_mode(path)
        .map(|mode| mode & 0o444 != 0)
        .unwrap_or(false)
}

/// Return whether the path should be treated as writable by the shell.
pub fn is_writable(path: &Path) -> bool {
    get_mode(path)
        .map(|mode| mode & 0o222 != 0)
        .unwrap_or(false)
}

/// Resolve both paths and report whether `candidate` remains under `root`.
///
/// Canonicalization resolves `..` components and symlinks before the prefix
/// comparison, so callers cannot validate a lexical path and then follow a
/// link outside the authority they were granted.
pub fn canonical_path_within(root: &Path, candidate: &Path) -> io::Result<bool> {
    let root = fs::canonicalize(root)?;
    let candidate = fs::canonicalize(candidate)?;
    Ok(candidate.starts_with(root))
}

/// Validate a not-yet-created leaf path against a canonical authority root.
///
/// A symlink destination normally does not exist yet, so canonicalizing the
/// full path would reject every legitimate creation. Canonicalize its parent
/// instead, after rejecting empty, dot, and traversal leaf names; this still
/// resolves every existing symlink and parent traversal before the check.
pub fn canonical_creation_path_within(root: &Path, candidate: &Path) -> io::Result<bool> {
    let Some(name) = candidate.file_name() else {
        return Ok(false);
    };
    if name == "." || name == ".." {
        return Ok(false);
    }
    let Some(parent) = candidate.parent() else {
        return Ok(false);
    };
    canonical_path_within(root, parent)
}

/// Resolve a filesystem path once at an authority boundary.
pub fn canonicalize_path(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

/// Create a symbolic link.
pub fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    {
        create_symlink_windows(target, link)
    }
}

#[cfg(windows)]
fn create_symlink_windows(target: &Path, link: &Path) -> io::Result<()> {
    let target_text = target.to_string_lossy();
    let looks_like_dir =
        target.is_dir() || target_text.ends_with('/') || target_text.ends_with('\\');

    let primary = if looks_like_dir {
        windows_symlink_flags(true)
    } else {
        windows_symlink_flags(false)
    };
    let secondary = if looks_like_dir {
        windows_symlink_flags(false)
    } else {
        windows_symlink_flags(true)
    };

    match create_symlink_windows_with_flags(target, link, primary) {
        Ok(()) => Ok(()),
        Err(primary_err) => match create_symlink_windows_with_flags(target, link, secondary) {
            Ok(()) => Ok(()),
            Err(_) => Err(primary_err),
        },
    }
}

#[cfg(windows)]
fn windows_symlink_flags(is_dir: bool) -> u32 {
    use windows_sys::Win32::Storage::FileSystem::{
        SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, SYMBOLIC_LINK_FLAG_DIRECTORY,
    };

    let mut flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
    if is_dir {
        flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
    }
    flags
}

#[cfg(windows)]
fn create_symlink_windows_with_flags(target: &Path, link: &Path, flags: u32) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::CreateSymbolicLinkW;

    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let link_wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();

    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers that live
    // for the duration of the call, and `flags` is composed from documented
    // `CreateSymbolicLinkW` flag constants.
    let result = unsafe { CreateSymbolicLinkW(link_wide.as_ptr(), target_wide.as_ptr(), flags) };
    if result {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Check if a path looks like a POSIX-style Windows path.
pub fn is_posix_windows_path(path: &str) -> bool {
    // MinGW style: /c/Users
    if let Some(rest) = path.strip_prefix('/') {
        if let Some(c) = rest.chars().next() {
            if c.is_ascii_alphabetic() {
                if let Some(s) = rest.chars().nth(1) {
                    if s == '/' || s == '\\' {
                        return true;
                    }
                }
            }
        }
    }

    // WSL2 style: /mnt/c/Users
    if path.starts_with("/mnt/") || path.starts_with("/MNT/") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that mutate the process-wide `MALT_SESSION_ID` env var must
    /// hold this lock for the duration of their mutation + assertions.
    /// Without it, `test_to_windows_path_tmp` and `test_malt_tmp_dir` can
    /// interleave under parallel test execution — one setting the var to
    /// "test-123" while the other expects "test-456", or reads it after
    /// the first test removed it. Caught the same way as the CWD_LOCK gaps
    /// in mash's test suite: intermittent failure only under parallel runs.
    #[cfg(windows)]
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_normalize_device_path() {
        assert_eq!(normalize_device_path("/dev/null"), "NUL");
        assert_eq!(normalize_device_path("/dev/stdin"), "CON");
        assert_eq!(normalize_device_path("/dev/stdout"), "CON");
        assert_eq!(normalize_device_path("/dev/tty"), "CON");
        assert_eq!(normalize_device_path("/dev/zero"), "NUL");
        assert_eq!(
            normalize_device_path("/some/other/path"),
            "/some/other/path"
        );
    }

    #[test]
    fn test_classify_virtual_dev() {
        assert_eq!(
            classify_virtual_dev("/dev/null"),
            Some(VirtualDevKind::Null)
        );
        assert_eq!(
            classify_virtual_dev("/dev/zero"),
            Some(VirtualDevKind::Zero)
        );
        assert_eq!(
            classify_virtual_dev("/dev/urandom"),
            Some(VirtualDevKind::Urandom)
        );
        assert_eq!(
            classify_virtual_dev("/dev/full"),
            Some(VirtualDevKind::Full)
        );
        assert_eq!(classify_virtual_dev("/dev/tty"), Some(VirtualDevKind::Tty));
        assert_eq!(
            classify_virtual_dev("/dev/stdin"),
            Some(VirtualDevKind::StdStream)
        );
        assert_eq!(classify_virtual_dev("/dev/fd/3"), Some(VirtualDevKind::Fd));
        assert_eq!(classify_virtual_dev("/home/user"), None);
    }

    #[test]
    fn test_try_drive_prefix() {
        assert_eq!(
            try_drive_prefix("c/Users/foo"),
            Some(PathBuf::from("C:\\Users\\foo"))
        );
        assert_eq!(
            try_drive_prefix("C/Users/foo"),
            Some(PathBuf::from("C:\\Users\\foo"))
        );
        assert_eq!(
            try_drive_prefix("d\\Projects"),
            Some(PathBuf::from("D:\\Projects"))
        );
        assert_eq!(try_drive_prefix("home/user"), None);
        assert_eq!(try_drive_prefix("123/abc"), None);
    }

    #[test]
    fn canonical_path_within_rejects_parent_escape() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        let outside = parent.path().join("outside.txt");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("inside.txt"), "inside").unwrap();
        fs::write(&outside, "outside").unwrap();

        assert!(canonical_path_within(&root, &root.join("inside.txt")).unwrap());
        assert!(!canonical_path_within(&root, &root.join("..").join("outside.txt")).unwrap());
    }

    #[test]
    fn canonical_creation_path_within_accepts_new_leaf_and_refuses_escape() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("root");
        fs::create_dir(&root).unwrap();

        assert!(canonical_creation_path_within(&root, &root.join("new-link")).unwrap());
        assert!(!canonical_creation_path_within(&root, &parent.path().join("new-link")).unwrap());
        assert!(!canonical_creation_path_within(&root, &root.join("..")).unwrap());
    }

    #[test]
    fn create_symlink_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "hello").unwrap();

        let result = create_symlink(&target, &link);

        #[cfg(unix)]
        {
            result.unwrap();
            assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        }

        #[cfg(windows)]
        {
            match result {
                Ok(()) => assert!(link.symlink_metadata().unwrap().file_type().is_symlink()),
                Err(err) => assert_eq!(err.raw_os_error(), Some(1314)),
            }
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_to_windows_path_mingw() {
        assert_eq!(
            to_windows_path(Path::new("/c/Users/foo")),
            PathBuf::from("C:\\Users\\foo")
        );
        assert_eq!(
            to_windows_path(Path::new("/D/Projects")),
            PathBuf::from("D:\\Projects")
        );
        assert_eq!(to_windows_path(Path::new("/c")), PathBuf::from("C:\\"));
    }

    #[test]
    #[cfg(windows)]
    fn test_to_windows_path_wsl() {
        assert_eq!(
            to_windows_path(Path::new("/mnt/c/Users/foo")),
            PathBuf::from("C:\\Users\\foo")
        );
        assert_eq!(
            to_windows_path(Path::new("/MNT/d/Projects")),
            PathBuf::from("D:\\Projects")
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_to_windows_path_tmp() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("MALT_SESSION_ID", "test-123");
        let tmp = to_windows_path(Path::new("/tmp"));
        assert!(tmp.to_string_lossy().contains("malt"));
        assert!(tmp.to_string_lossy().contains("test-123"));

        let tmp_file = to_windows_path(Path::new("/tmp/file.txt"));
        assert!(tmp_file.to_string_lossy().contains("malt"));
        assert!(tmp_file.to_string_lossy().contains("file.txt"));

        // Don't leak MALT_SESSION_ID into whichever test runs next.
        std::env::remove_var("MALT_SESSION_ID");
    }

    #[test]
    #[cfg(windows)]
    fn test_to_windows_path_already_windows() {
        // Already Windows paths should be unchanged
        assert_eq!(
            to_windows_path(Path::new("C:\\Users\\foo")),
            PathBuf::from("C:\\Users\\foo")
        );
        assert_eq!(
            to_windows_path(Path::new("\\\\server\\share")),
            PathBuf::from("\\\\server\\share")
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_to_windows_path_passthrough() {
        // Non-Windows-style paths should be unchanged
        assert_eq!(
            to_windows_path(Path::new("/home/user")),
            PathBuf::from("/home/user")
        );
        assert_eq!(
            to_windows_path(Path::new("/usr/bin")),
            PathBuf::from("/usr/bin")
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_malt_tmp_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("MALT_SESSION_ID", "test-456");
        let tmp = malt_tmp_dir();
        assert!(tmp.to_string_lossy().contains("malt"));
        assert!(tmp.to_string_lossy().contains("test-456"));

        // Standalone mode
        std::env::remove_var("MALT_SESSION_ID");
        let tmp = malt_tmp_dir();
        assert!(tmp.to_string_lossy().contains("malt"));
        assert!(tmp.to_string_lossy().contains("standalone-"));
    }

    #[test]
    fn test_is_posix_windows_path() {
        assert!(is_posix_windows_path("/c/Users/foo"));
        assert!(is_posix_windows_path("/C/Users/foo"));
        assert!(is_posix_windows_path("/mnt/c/Users"));
        assert!(is_posix_windows_path("/MNT/d/Projects"));

        assert!(!is_posix_windows_path("/home/user"));
        assert!(!is_posix_windows_path("/usr/bin"));
        assert!(!is_posix_windows_path("C:\\Users"));
    }

    #[test]
    #[cfg(windows)]
    fn test_permission_overlay_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.txt");
        fs::write(&path, "hello").unwrap();

        set_mode(&path, 0o333).unwrap();
        assert_eq!(get_mode(&path).unwrap(), 0o333);
        assert!(!is_readable(&path));
        assert!(is_writable(&path));

        set_mode(&path, 0o444).unwrap();
        assert_eq!(get_mode(&path).unwrap(), 0o444);
        assert!(is_readable(&path));
        assert!(!is_writable(&path));
    }
}
