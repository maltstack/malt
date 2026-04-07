//! Virtual File System for POSIX device emulation on Windows.
//!
//! Provides Windows-compatible implementations of POSIX device files:
//! - `/dev/null` - discards writes, returns EOF on read
//! - `/dev/zero` - returns infinite zero bytes on read
//! - `/dev/urandom` - returns random bytes on read
//! - `/dev/full` - returns zeros on read, fails on write (ENOSPC)
//! - `/dev/fd/N` - file descriptor access via FdRegistry

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

// Import the fd module
pub mod fd;

// Re-export key types
pub use fd::{open_dev_fd, open_dev_fd_read, open_dev_fd_write, FdRegistry, SharedFdRegistry};

/// Virtual device types that need special handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualDevKind {
    /// `/dev/null` - discards all writes, returns EOF on read
    Null,
    /// `/dev/zero` - returns infinite zero bytes on read
    Zero,
    /// `/dev/urandom`, `/dev/random` - returns random bytes on read
    Urandom,
    /// `/dev/full` - returns zeros on read, fails on write
    Full,
    /// `/dev/tty` - console I/O
    Tty,
    /// `/dev/stdin`, `/dev/stdout`, `/dev/stderr` - standard streams
    StdStream,
    /// `/dev/fd/N` - file descriptor access
    Fd,
}

/// Open mode for virtual devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevOpenMode {
    Read,
    Write,
    ReadWrite,
}

/// Classify a path as a virtual device.
///
/// Returns `Some(VirtualDevKind)` if the path is a POSIX virtual device
/// that needs emulation on Windows.
pub fn classify_virtual_dev(path: &str) -> Option<VirtualDevKind> {
    let normalized = normalize_dev_path_str(path);

    match normalized.as_str() {
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

/// Normalize a device path for comparison.
///
/// On Windows, this handles `NUL`, `CON`, etc.
fn normalize_dev_path_str(path: &str) -> String {
    #[cfg(windows)]
    {
        let upper = path.to_ascii_uppercase();
        match upper.as_str() {
            "NUL" => "/dev/null".to_string(),
            "CON" => "/dev/tty".to_string(),
            _ => path.to_string(),
        }
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

/// Try to open a path as a virtual device.
///
/// Returns `Some(File)` if the path is a virtual device and can be opened
/// with the given mode. Returns `None` if the path is not a virtual device
/// or cannot be opened (e.g., write to `/dev/full`).
///
/// For regular files, use `File::open()` or `OpenOptions::new()`.
pub fn try_open_virtual_dev(path: &Path, mode: DevOpenMode) -> Option<std::io::Result<File>> {
    let path_str = path.to_string_lossy();
    let kind = classify_virtual_dev(&path_str)?;

    Some(open_virtual_dev(kind, mode))
}

/// Open a virtual device with the specified mode.
///
/// Returns `Ok(File)` on success, `Err` if the device cannot be opened
/// (e.g., write to `/dev/full` returns ENOSPC error).
///
/// For `/dev/fd/N`, use [`open_dev_fd`] instead which requires a registry.
pub fn open_virtual_dev(kind: VirtualDevKind, mode: DevOpenMode) -> std::io::Result<File> {
    match kind {
        VirtualDevKind::Null => open_dev_null(mode),
        VirtualDevKind::Zero => open_dev_zero(mode),
        VirtualDevKind::Urandom => open_dev_urandom(mode),
        VirtualDevKind::Full => open_dev_full(mode),
        VirtualDevKind::Tty => open_dev_tty(mode),
        VirtualDevKind::StdStream => open_dev_std_stream(mode),
        VirtualDevKind::Fd => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "/dev/fd/N requires FdRegistry - use open_dev_fd() instead",
        )),
    }
}

/// Open `/dev/null` - the null device.
///
/// - Read: returns EOF immediately (Windows `NUL`)
/// - Write: discards all data (Windows `NUL`)
fn open_dev_null(mode: DevOpenMode) -> io::Result<File> {
    use std::fs::OpenOptions;

    match mode {
        DevOpenMode::Read => OpenOptions::new().read(true).open("NUL"),
        DevOpenMode::Write => OpenOptions::new().write(true).open("NUL"),
        DevOpenMode::ReadWrite => OpenOptions::new().read(true).write(true).open("NUL"),
    }
}

/// Open `/dev/zero` - returns infinite zero bytes on read.
///
/// - Read: returns infinite `\0` bytes via pipe + thread
/// - Write: returns error (not supported)
fn open_dev_zero(mode: DevOpenMode) -> io::Result<File> {
    match mode {
        DevOpenMode::Read => {
            let (reader, writer) = crate::io::create_pipe()?;

            std::thread::Builder::new()
                .name("malt-dev-zero".into())
                .spawn(move || {
                    let buf = [0u8; 8192];
                    let mut w = writer;
                    loop {
                        if w.write_all(&buf).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            Ok(reader)
        }
        DevOpenMode::Write | DevOpenMode::ReadWrite => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "/dev/zero: write not supported",
        )),
    }
}

/// Open `/dev/urandom` - returns random bytes on read.
///
/// - Read: returns random bytes via pipe + thread (Windows CNG)
/// - Write: returns error (not supported)
fn open_dev_urandom(mode: DevOpenMode) -> io::Result<File> {
    match mode {
        DevOpenMode::Read => {
            #[cfg(windows)]
            {
                open_dev_urandom_windows()
            }
            #[cfg(not(windows))]
            {
                use std::fs::OpenOptions;
                OpenOptions::new().read(true).open("/dev/urandom")
            }
        }
        DevOpenMode::Write | DevOpenMode::ReadWrite => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "/dev/urandom: write not supported",
        )),
    }
}

#[cfg(windows)]
fn open_dev_urandom_windows() -> io::Result<File> {
    let (reader, writer) = crate::io::create_pipe()?;

    std::thread::Builder::new()
        .name("malt-dev-urandom".into())
        .spawn(move || {
            let mut w = writer;
            let mut buf = [0u8; 4096];

            loop {
                if !gen_random_bytes(&mut buf) {
                    break;
                }

                if w.write_all(&buf).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    Ok(reader)
}

#[cfg(windows)]
fn gen_random_bytes(buf: &mut [u8]) -> bool {
    use std::ptr;
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    let result = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };

    result != 0
}

/// Open `/dev/full` - returns zeros on read, fails on write.
///
/// - Read: returns zeros (like `/dev/zero`)
/// - Write: returns ENOSPC error
fn open_dev_full(mode: DevOpenMode) -> io::Result<File> {
    match mode {
        DevOpenMode::Read => open_dev_zero(DevOpenMode::Read),
        DevOpenMode::Write | DevOpenMode::ReadWrite => Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "/dev/full: No space left on device",
        )),
    }
}

/// Open `/dev/tty` - console I/O.
///
/// On Windows, maps to `CON`.
fn open_dev_tty(mode: DevOpenMode) -> io::Result<File> {
    use std::fs::OpenOptions;

    match mode {
        DevOpenMode::Read => OpenOptions::new().read(true).open("CON"),
        DevOpenMode::Write => OpenOptions::new().write(true).open("CON"),
        DevOpenMode::ReadWrite => OpenOptions::new().read(true).write(true).open("CON"),
    }
}

/// Open `/dev/stdin`, `/dev/stdout`, `/dev/stderr`.
///
/// These redirect to the console (`CON`) on Windows.
fn open_dev_std_stream(_mode: DevOpenMode) -> io::Result<File> {
    use std::fs::OpenOptions;

    OpenOptions::new().read(true).write(true).open("CON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

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
            classify_virtual_dev("/dev/random"),
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
    fn test_dev_null() {
        let mut file = open_virtual_dev(VirtualDevKind::Null, DevOpenMode::Write).unwrap();
        assert!(file.write_all(b"hello").is_ok());
    }

    #[test]
    fn test_dev_null_read() {
        let mut file = open_virtual_dev(VirtualDevKind::Null, DevOpenMode::Read).unwrap();
        let mut buf = [0u8; 10];
        let n = file.read(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_dev_zero_write_fails() {
        let result = open_virtual_dev(VirtualDevKind::Zero, DevOpenMode::Write);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_dev_full_write_fails() {
        let result = open_virtual_dev(VirtualDevKind::Full, DevOpenMode::Write);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::StorageFull);
    }

    #[test]
    fn test_dev_urandom_write_fails() {
        let result = open_virtual_dev(VirtualDevKind::Urandom, DevOpenMode::Write);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn test_try_open_virtual_dev() {
        let result = try_open_virtual_dev(Path::new("/dev/null"), DevOpenMode::Write);
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());

        let result = try_open_virtual_dev(Path::new("/home/user"), DevOpenMode::Read);
        assert!(result.is_none());
    }
}
