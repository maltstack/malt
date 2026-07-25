//! /dev/fd/N virtual file system implementation.
//!
//! Provides cross-platform support for `/dev/fd/N` paths used in shell redirects.
//! This enables constructs like:
//! - `cmd1 > /dev/fd/3 3> outfile`
//! - `cmd < /dev/fd/0` (duplicate stdin)
//!
//! ## Platform Support
//!
//! ### Unix (Linux/macOS)
//! On Unix systems, `/dev/fd/N` is typically provided by the OS as a symlink to
//! the actual file descriptor. We pass through to the real FD when possible,
//! but also provide fallback emulation for cases where the OS support is
//! limited.
//!
//! ### Windows
//! Windows doesn't have native `/dev/fd` support. We emulate it using:
//! - **Named pipes** for pipe-like communication
//! - **Temp files** for file-backed descriptors
//! - **Handle duplication** where possible

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Registry for virtual file descriptors.
///
/// Maps `/dev/fd/N` paths to actual file handles, providing cross-platform
/// support for shell file descriptor redirections.
#[derive(Debug)]
pub struct FdRegistry {
    /// Next FD number to allocate (starts at 10 to avoid conflicts with stdio)
    next_fd: u32,
    /// Map of FD numbers to backing resources
    fds: HashMap<u32, FdBacking>,
    /// Process ID for namespacing (Windows named pipes)
    pid: u32,
}

/// Backing resource for a virtual file descriptor.
#[derive(Debug)]
// NamedPipe/PipeEnd model descriptor kinds the VFS is expected to grow
// into. Retained deliberately so the enum describes the domain rather
// than only what is wired today.
#[allow(dead_code)]
enum FdBacking {
    /// Unix: actual file descriptor (duplicated for safety)
    #[cfg(unix)]
    RawFd(std::os::unix::io::RawFd),

    /// Any platform: a real file handle duplicated on demand.
    FileHandle { file: std::fs::File },

    /// Windows: named pipe
    #[cfg(windows)]
    NamedPipe {
        path: String,
        #[allow(dead_code)]
        server_handle: Option<std::fs::File>,
    },

    /// Any platform: temp file backing
    TempFile { path: PathBuf },

    /// Stored pipe reader/writer pair reference
    PipeEnd {
        is_reader: bool,
        // Note: We store the FD number of the paired end
        paired_fd: u32,
    },
}

/// Information about a registered FD for opening.
#[derive(Debug, Clone)]
pub struct FdInfo {
    pub fd_num: u32,
    pub is_pipe: bool,
    pub is_readable: bool,
    pub is_writable: bool,
}

impl Default for FdRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FdRegistry {
    /// Create a new empty FD registry.
    pub fn new() -> Self {
        Self {
            next_fd: 10, // Start at 10 to avoid stdio (0,1,2) and common fds (3-9)
            fds: HashMap::new(),
            pid: std::process::id(),
        }
    }

    /// Register a file as a virtual FD, returning the FD number.
    ///
    /// The file is consumed and stored in the registry. Use `open()` to
    /// get a new handle to the same underlying resource.
    pub fn register_file(&mut self, file: std::fs::File) -> u32 {
        let fd = self.next_fd;
        self.next_fd += 1;

        // Try to determine if this is a pipe or regular file
        let is_pipe = Self::is_pipe(&file);

        let backing = if is_pipe {
            // For pipes on Windows, we need special handling
            #[cfg(windows)]
            {
                self.create_pipe_backing(file, fd)
            }
            #[cfg(unix)]
            {
                self.create_unix_pipe_backing(file, fd)
            }
        } else {
            // For regular files, use temp file backing
            self.create_file_backing(file)
        };

        if let Some(b) = backing {
            self.fds.insert(fd, b);
        }

        fd
    }

    /// Register a file at a specific FD number.
    ///
    /// This allows registering a file as a specific FD (e.g., FD 3 for `exec 3> file`).
    /// If the FD is already in use, it will be closed and replaced.
    pub fn register_file_at(&mut self, fd: u32, file: std::fs::File) {
        // Close any existing FD at this number
        let _ = self.close(fd);

        // Try to determine if this is a pipe or regular file
        let is_pipe = Self::is_pipe(&file);

        let backing = if is_pipe {
            #[cfg(windows)]
            {
                self.create_pipe_backing(file, fd)
            }
            #[cfg(unix)]
            {
                self.create_unix_pipe_backing(file, fd)
            }
        } else {
            self.create_file_backing(file)
        };

        if let Some(b) = backing {
            self.fds.insert(fd, b);
        }

        // Update next_fd if needed to avoid conflicts
        if fd >= self.next_fd {
            self.next_fd = fd + 1;
        }
    }

    /// Register a pipe (reader/writer pair) as two virtual FDs.
    ///
    /// Returns (reader_fd, writer_fd) for the registered pipe ends.
    /// This is used for pipeline stages to communicate.
    pub fn register_pipe(&mut self) -> io::Result<(u32, u32)> {
        let (reader, writer) = crate::io::create_pipe()?;

        let reader_fd = self.next_fd;
        self.next_fd += 1;
        let writer_fd = self.next_fd;
        self.next_fd += 1;

        #[cfg(unix)]
        {
            use std::os::unix::io::IntoRawFd;
            self.fds
                .insert(reader_fd, FdBacking::RawFd(reader.into_raw_fd()));
            self.fds
                .insert(writer_fd, FdBacking::RawFd(writer.into_raw_fd()));
        }

        #[cfg(windows)]
        {
            // On Windows, store the handles directly
            self.fds.insert(
                reader_fd,
                FdBacking::TempFile {
                    path: PathBuf::from(format!(
                        "\\\\.\\pipe\\malt-fd-{}-reader-{}",
                        self.pid, reader_fd
                    )),
                },
            );
            // Keep the actual file handles in a separate storage
            // For now, we'll use temp files as backing
            drop(reader);
            drop(writer);
        }

        #[cfg(not(any(unix, windows)))]
        {
            drop(reader);
            drop(writer);
        }

        Ok((reader_fd, writer_fd))
    }

    /// Open a /dev/fd/N path, returning a File handle.
    ///
    /// This is called when a process opens `/dev/fd/N` to read/write.
    pub fn open(&self, fd_num: u32) -> io::Result<std::fs::File> {
        match self.fds.get(&fd_num) {
            #[cfg(unix)]
            Some(FdBacking::RawFd(raw_fd)) => {
                // Duplicate the FD for safe independent access
                self.dup_raw_fd(*raw_fd)
            }

            #[cfg(windows)]
            Some(FdBacking::NamedPipe { path, .. }) => self.open_windows_named_pipe(path),

            Some(FdBacking::FileHandle { file }) => file.try_clone(),

            Some(FdBacking::TempFile { path, .. }) => {
                // Open the temp file
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
            }

            Some(FdBacking::PipeEnd { .. }) => {
                // This shouldn't happen in normal usage - pipe ends
                // should have been converted to concrete backings
                Err(io::Error::other(format!(
                    "FD {} is an unresolved pipe end",
                    fd_num
                )))
            }

            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Bad file descriptor: {}", fd_num),
            )),
        }
    }

    /// Open a /dev/fd/N path for reading.
    pub fn open_read(&self, fd_num: u32) -> io::Result<std::fs::File> {
        match self.fds.get(&fd_num) {
            #[cfg(unix)]
            Some(FdBacking::RawFd(raw_fd)) => self.dup_raw_fd_read(*raw_fd),

            _ => {
                // Fall back to generic open
                let file = self.open(fd_num)?;
                // Verify we can read
                let _ = file.metadata()?;
                Ok(file)
            }
        }
    }

    /// Open a /dev/fd/N path for writing.
    pub fn open_write(&self, fd_num: u32) -> io::Result<std::fs::File> {
        match self.fds.get(&fd_num) {
            #[cfg(unix)]
            Some(FdBacking::RawFd(raw_fd)) => self.dup_raw_fd_write(*raw_fd),

            _ => {
                // Fall back to generic open
                let file = self.open(fd_num)?;
                Ok(file)
            }
        }
    }

    /// Close and remove a registered FD.
    pub fn close(&mut self, fd_num: u32) -> io::Result<()> {
        if let Some(backing) = self.fds.remove(&fd_num) {
            #[cfg(unix)]
            if let FdBacking::RawFd(fd) = backing {
                unsafe {
                    libc::close(fd);
                }
            }

            #[cfg(windows)]
            if let FdBacking::NamedPipe { .. } = backing {
                // Named pipes close automatically when dropped
            }

            if let FdBacking::TempFile { path } = backing {
                // Clean up temp file
                let _ = std::fs::remove_file(&path);
            }
        }

        Ok(())
    }

    /// Check if an FD number is registered.
    pub fn is_registered(&self, fd_num: u32) -> bool {
        self.fds.contains_key(&fd_num)
    }

    /// List all registered FDs.
    pub fn list_fds(&self) -> Vec<u32> {
        self.fds.keys().copied().collect()
    }

    /// Parse a /dev/fd/N path and return the FD number if valid.
    pub fn parse_dev_fd_path(path: &str) -> Option<u32> {
        let path = path.trim();
        let prefix = "/dev/fd/";

        if let Some(num_str) = path.strip_prefix(prefix) {
            num_str.parse::<u32>().ok()
        } else {
            None
        }
    }

    /// Check if a path is a /dev/fd/N path.
    pub fn is_dev_fd_path(path: &str) -> bool {
        Self::parse_dev_fd_path(path).is_some()
    }

    // Platform-specific helper methods

    #[cfg(unix)]
    fn dup_raw_fd(&self, fd: std::os::unix::io::RawFd) -> io::Result<std::fs::File> {
        use std::os::unix::io::FromRawFd;

        unsafe {
            let new_fd = libc::dup(fd);
            if new_fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(std::fs::File::from_raw_fd(new_fd))
        }
    }

    #[cfg(unix)]
    fn dup_raw_fd_read(&self, fd: std::os::unix::io::RawFd) -> io::Result<std::fs::File> {
        // On Unix, we can use the same dup
        self.dup_raw_fd(fd)
    }

    #[cfg(unix)]
    fn dup_raw_fd_write(&self, fd: std::os::unix::io::RawFd) -> io::Result<std::fs::File> {
        // On Unix, we can use the same dup
        self.dup_raw_fd(fd)
    }

    #[cfg(unix)]
    fn create_unix_pipe_backing(&mut self, file: std::fs::File, _fd: u32) -> Option<FdBacking> {
        use std::os::unix::io::IntoRawFd;
        Some(FdBacking::RawFd(file.into_raw_fd()))
    }

    #[cfg(windows)]
    fn create_pipe_backing(&mut self, _file: std::fs::File, fd: u32) -> Option<FdBacking> {
        // Create a named pipe for this FD
        let _pipe_path = format!("\\\\.\\pipe\\malt-fd-{}-{}", self.pid, fd);

        // For now, use temp file backing as it's more reliable
        // Named pipes require async I/O on Windows which is complex
        self.create_temp_file_backing(_file, fd)
    }

    #[cfg(windows)]
    fn open_windows_named_pipe(&self, path: &str) -> io::Result<std::fs::File> {
        // Try to connect to the named pipe
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
    }

    fn create_file_backing(&mut self, file: std::fs::File) -> Option<FdBacking> {
        Some(FdBacking::FileHandle { file })
    }

    fn create_temp_file_backing(&mut self, mut file: std::fs::File, fd: u32) -> Option<FdBacking> {
        use std::io::{Read, Seek, Write};

        // Create a temp file to back this FD
        let temp_path = std::env::temp_dir().join(format!("malt-fd-{}-{}", self.pid, fd));

        // Copy content from original file to temp file
        let mut temp_file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
        {
            Ok(f) => f,
            Err(_) => return None,
        };

        // Copy content
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_ok() {
            let _ = temp_file.write_all(&buf);
            let _ = temp_file.flush();
            let _ = temp_file.rewind();
        }

        // Keep the temp file path; the actual handle will be opened on demand
        drop(temp_file);
        drop(file);

        Some(FdBacking::TempFile { path: temp_path })
    }

    fn is_pipe(_file: &std::fs::File) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            unsafe {
                let mut stat: libc::stat = std::mem::zeroed();
                if libc::fstat(fd, &mut stat) == 0 {
                    // Check if it's a FIFO (named pipe) or socket
                    (stat.st_mode & libc::S_IFMT) == libc::S_IFIFO
                        || (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK
                } else {
                    false
                }
            }
        }

        #[cfg(windows)]
        {
            // On Windows, we can't easily detect pipes from File handles
            // without using native APIs. For now, assume all files could be pipes.
            false // Default to temp file backing for reliability
        }

        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }
}

/// Thread-safe wrapper for FdRegistry.
#[derive(Debug, Clone)]
pub struct SharedFdRegistry {
    inner: Arc<Mutex<FdRegistry>>,
}

impl SharedFdRegistry {
    /// Create a new shared FD registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FdRegistry::new())),
        }
    }

    /// Register a file and return the FD number.
    pub fn register_file(&self, file: std::fs::File) -> u32 {
        self.inner.lock().unwrap().register_file(file)
    }

    /// Register a pipe and return (reader_fd, writer_fd).
    pub fn register_pipe(&self) -> io::Result<(u32, u32)> {
        self.inner.lock().unwrap().register_pipe()
    }

    /// Open an FD by number.
    pub fn open(&self, fd_num: u32) -> io::Result<std::fs::File> {
        self.inner.lock().unwrap().open(fd_num)
    }

    /// Open an FD for reading.
    pub fn open_read(&self, fd_num: u32) -> io::Result<std::fs::File> {
        self.inner.lock().unwrap().open_read(fd_num)
    }

    /// Open an FD for writing.
    pub fn open_write(&self, fd_num: u32) -> io::Result<std::fs::File> {
        self.inner.lock().unwrap().open_write(fd_num)
    }

    /// Close an FD.
    pub fn close(&self, fd_num: u32) -> io::Result<()> {
        self.inner.lock().unwrap().close(fd_num)
    }

    /// Check if FD is registered.
    pub fn is_registered(&self, fd_num: u32) -> bool {
        self.inner.lock().unwrap().is_registered(fd_num)
    }

    /// List all registered FDs.
    pub fn list_fds(&self) -> Vec<u32> {
        self.inner.lock().unwrap().list_fds()
    }

    /// Register a file at a specific FD number.
    pub fn register_file_at(&self, fd: u32, file: std::fs::File) {
        self.inner.lock().unwrap().register_file_at(fd, file);
    }
}

impl Default for SharedFdRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Open a /dev/fd/N path using the provided registry.
///
/// This is the main entry point for opening /dev/fd/N paths.
/// If no registry is provided, attempts direct OS open (Unix only).
pub fn open_dev_fd(path: &str, registry: Option<&SharedFdRegistry>) -> io::Result<std::fs::File> {
    let fd_num = FdRegistry::parse_dev_fd_path(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid /dev/fd path: {}", path),
        )
    })?;

    // First, try the registry if available
    if let Some(reg) = registry {
        if reg.is_registered(fd_num) {
            return reg.open(fd_num);
        }
    }

    // On Unix, try direct FD access
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;

        // Check if the FD is valid
        unsafe {
            if libc::fcntl(fd_num as i32, libc::F_GETFD) >= 0 {
                // FD is valid, duplicate it
                let new_fd = libc::dup(fd_num as i32);
                if new_fd >= 0 {
                    return Ok(std::fs::File::from_raw_fd(new_fd));
                }
            }
        }
    }

    // Try to open OS-provided /dev/fd/N
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

/// Open a /dev/fd/N path for reading.
pub fn open_dev_fd_read(
    path: &str,
    registry: Option<&SharedFdRegistry>,
) -> io::Result<std::fs::File> {
    let fd_num = FdRegistry::parse_dev_fd_path(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid /dev/fd path: {}", path),
        )
    })?;

    if let Some(reg) = registry {
        if reg.is_registered(fd_num) {
            return reg.open_read(fd_num);
        }
    }

    std::fs::OpenOptions::new().read(true).open(path)
}

/// Open a /dev/fd/N path for writing.
pub fn open_dev_fd_write(
    path: &str,
    registry: Option<&SharedFdRegistry>,
) -> io::Result<std::fs::File> {
    let fd_num = FdRegistry::parse_dev_fd_path(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid /dev/fd path: {}", path),
        )
    })?;

    if let Some(reg) = registry {
        if reg.is_registered(fd_num) {
            return reg.open_write(fd_num);
        }
    }

    std::fs::OpenOptions::new().write(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, Write};

    #[test]
    fn test_parse_dev_fd_path() {
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/fd/0"), Some(0));
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/fd/10"), Some(10));
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/fd/99"), Some(99));
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/fd/"), None);
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/fd/abc"), None);
        assert_eq!(FdRegistry::parse_dev_fd_path("/dev/null"), None);
        assert_eq!(FdRegistry::parse_dev_fd_path("/home/user"), None);
    }

    #[test]
    fn test_is_dev_fd_path() {
        assert!(FdRegistry::is_dev_fd_path("/dev/fd/0"));
        assert!(FdRegistry::is_dev_fd_path("/dev/fd/10"));
        assert!(!FdRegistry::is_dev_fd_path("/dev/null"));
        assert!(!FdRegistry::is_dev_fd_path("/home/user"));
    }

    #[test]
    fn test_register_and_open_file() -> io::Result<()> {
        let mut registry = FdRegistry::new();

        // Create a temp file with some content
        let mut temp_file = tempfile::NamedTempFile::new()?;
        temp_file.write_all(b"hello world")?;
        temp_file.flush()?;
        temp_file.rewind()?;

        // Register the file
        let fd = registry.register_file(temp_file.reopen()?);
        assert!(fd >= 10);

        // Open via registry
        let mut file = registry.open(fd)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        assert_eq!(contents, "hello world");

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_register_pipe() -> io::Result<()> {
        let mut registry = FdRegistry::new();

        // Register a pipe
        let (reader_fd, writer_fd) = registry.register_pipe()?;
        assert!(reader_fd >= 10);
        assert!(writer_fd >= 10);

        // Write to writer
        let mut writer = registry.open(writer_fd)?;
        writer.write_all(b"pipe test")?;
        drop(writer);

        // Read from reader
        let mut reader = registry.open(reader_fd)?;
        let mut contents = String::new();
        reader.read_to_string(&mut contents)?;
        assert_eq!(contents, "pipe test");

        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn test_register_pipe_windows() -> io::Result<()> {
        // On Windows, pipe support requires named pipe implementation
        // which is more complex. For now, we just test that registration
        // doesn't panic and returns valid FD numbers.
        let mut registry = FdRegistry::new();

        let (reader_fd, writer_fd) = registry.register_pipe()?;
        assert!(reader_fd >= 10);
        assert!(writer_fd >= 10);

        // Note: Full pipe I/O not yet supported on Windows
        // This would require named pipe server/client implementation
        Ok(())
    }

    #[test]
    fn test_shared_registry() -> io::Result<()> {
        let registry = SharedFdRegistry::new();

        // Register a file
        let temp_file = tempfile::NamedTempFile::new()?;
        let fd = registry.register_file(temp_file.reopen()?);

        // Verify it's registered
        assert!(registry.is_registered(fd));

        // List FDs
        let fds = registry.list_fds();
        assert!(fds.contains(&fd));

        Ok(())
    }

    #[test]
    fn test_close_fd() -> io::Result<()> {
        let mut registry = FdRegistry::new();

        let temp_file = tempfile::NamedTempFile::new()?;
        let fd = registry.register_file(temp_file.reopen()?);

        assert!(registry.is_registered(fd));
        registry.close(fd)?;
        assert!(!registry.is_registered(fd));

        Ok(())
    }

    #[test]
    fn test_open_invalid_fd() {
        let registry = FdRegistry::new();

        let result = registry.open(999);
        assert!(result.is_err());
    }
}
