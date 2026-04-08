//! Process spawning — SpawnConfig, Child, spawn().
//!
//! This module is the single gateway for creating child processes in MALT.
//! It wraps `std::process::Command` with platform-specific extensions:
//! - Unix: `pre_exec` for process group setup (`setpgid`), zombie reaping via `waitpid`
//! - Windows: `creation_flags(CREATE_NEW_PROCESS_GROUP)`, handle duplication, `WaitForSingleObject`
//!
//! After spawning, the `std::process::Child` is forgotten and the process is
//! managed directly via OS primitives (pid on Unix, HANDLE on Windows).

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;
use std::path::PathBuf;
use thiserror::Error;

// ── Error type ──────────────────────────────────────────────────────────

/// Errors that can occur when spawning a child process.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SpawnError {
    /// The executable was not found on disk.
    #[error("executable not found: {path}")]
    NotFound { path: PathBuf },

    /// The caller lacks permission to execute the program.
    #[error("permission denied: {path}")]
    PermissionDenied { path: PathBuf },

    /// The requested process group ID overflows `i32`.
    #[error("pgid {pgid} overflows i32 (valid range: 1..=2147483647)")]
    InvalidPgid { pgid: u32 },

    /// An OS-level I/O error occurred.
    #[error("spawn failed: {0}")]
    Io(#[from] std::io::Error),
}

// ── Io ──────────────────────────────────────────────────────────────────

/// How a file descriptor / handle is connected to the child.
///
/// # Panics
///
/// Cloning an `Io::File` calls `File::try_clone()` (which calls `dup(2)` /
/// `DuplicateHandle`). If the process has exhausted its file-descriptor limit,
/// the clone will panic.
#[derive(Debug)]
#[non_exhaustive]
pub enum Io {
    /// Inherit the parent's corresponding fd / handle.
    Inherit,
    /// Connect to the platform null device (`/dev/null` or `NUL`).
    Null,
    /// Create a new pipe; the parent gets the other end.
    Pipe,
    /// Connect the child to an existing open file.
    File(std::fs::File),
    /// Connect the child to a raw fd (Unix only).
    #[cfg(unix)]
    Fd(std::os::unix::io::RawFd),
    /// Connect the child to a raw handle (Windows only).
    #[cfg(windows)]
    Handle(::windows_sys::Win32::Foundation::HANDLE),
}

impl Clone for Io {
    fn clone(&self) -> Self {
        match self {
            Io::Inherit => Io::Inherit,
            Io::Null => Io::Null,
            Io::Pipe => Io::Pipe,
            Io::File(f) => {
                let cloned = match f.try_clone() {
                    Ok(dup) => dup,
                    Err(e) => panic!("Io::File clone failed to dup fd: {e}"),
                };
                Io::File(cloned)
            }
            #[cfg(unix)]
            Io::Fd(fd) => Io::Fd(*fd),
            #[cfg(windows)]
            Io::Handle(h) => Io::Handle(*h),
        }
    }
}

impl PartialEq for Io {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Io::Inherit, Io::Inherit) | (Io::Null, Io::Null) | (Io::Pipe, Io::Pipe) => true,
            (Io::File(a), Io::File(b)) => {
                #[cfg(unix)]
                {
                    use std::os::unix::io::AsRawFd;
                    a.as_raw_fd() == b.as_raw_fd()
                }
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    a.as_raw_handle() == b.as_raw_handle()
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = (a, b);
                    false
                }
            }
            #[cfg(unix)]
            (Io::Fd(a), Io::Fd(b)) => a == b,
            #[cfg(windows)]
            (Io::Handle(a), Io::Handle(b)) => a == b,
            _ => false,
        }
    }
}

// ── ProcessGroup ────────────────────────────────────────────────────────

/// Process group configuration for a spawned child.
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub enum ProcessGroup {
    /// Inherit the parent's process group.
    #[default]
    Inherit,
    /// Create a new process group with the child as leader.
    New,
    /// Join an existing process group by PGID.
    Join(u32),
}

// ── ExitStatus ──────────────────────────────────────────────────────────

/// Exit status of a completed child process.
///
/// On Unix, a value of 128+N indicates the child was killed by signal N.
/// On Windows, the value is the process exit code (unsigned, stored as i32).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(i32);

impl ExitStatus {
    /// Create an `ExitStatus` from a raw code.
    pub fn from_raw(code: i32) -> Self {
        Self(code)
    }

    /// The raw exit code.
    pub fn code(&self) -> i32 {
        self.0
    }

    /// Whether the process exited successfully (code 0).
    pub fn success(&self) -> bool {
        self.0 == 0
    }

    /// On Unix, if the process was killed by a signal, return the signal number.
    /// Returns `None` if the process exited normally or on Windows.
    #[cfg(unix)]
    pub fn signal(&self) -> Option<i32> {
        if self.0 > 128 {
            Some(self.0 - 128)
        } else {
            None
        }
    }
}

impl std::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

// ── SpawnConfig ─────────────────────────────────────────────────────────

/// Configuration for spawning a child process.
pub struct SpawnConfig {
    /// The program to execute.
    pub program: PathBuf,
    /// Command-line arguments (not including argv[0]).
    pub args: Vec<OsString>,
    /// Environment variables to set in the child. Applied on top of the
    /// inherited environment unless `env_clear` is set.
    pub env: Vec<(OsString, OsString)>,
    /// If true, clear the child's environment before applying `env`.
    pub env_clear: bool,
    /// Working directory for the child. `None` inherits the parent's cwd.
    pub cwd: Option<PathBuf>,
    /// How to wire the child's stdin.
    pub stdin: Io,
    /// How to wire the child's stdout.
    pub stdout: Io,
    /// How to wire the child's stderr.
    pub stderr: Io,
    /// Process group configuration.
    pub process_group: ProcessGroup,
    /// Override argv[0] (the process name as seen by the child). When `None`,
    /// argv[0] defaults to the program path. On Unix, uses `Command::arg0()`.
    pub argv0: Option<OsString>,
    /// Additional file descriptors to map into the child process on Unix.
    /// Each tuple is `(target_fd, source_file)`.
    #[cfg(unix)]
    pub extra_fds: Vec<(i32, std::fs::File)>,
    /// File descriptors to close in the child process on Unix before `exec`.
    #[cfg(unix)]
    pub close_fds: Vec<i32>,
}

impl SpawnConfig {
    /// Create a minimal spawn configuration for the given program.
    ///
    /// Defaults: inherit stdio, inherit process group, no env changes.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            env_clear: false,
            cwd: None,
            stdin: Io::Inherit,
            stdout: Io::Inherit,
            stderr: Io::Inherit,
            process_group: ProcessGroup::Inherit,
            argv0: None,
            #[cfg(unix)]
            extra_fds: Vec::new(),
            #[cfg(unix)]
            close_fds: Vec::new(),
        }
    }

    /// Add a single argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add multiple arguments.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable.
    pub fn env(mut self, key: impl Into<OsString>, val: impl Into<OsString>) -> Self {
        self.env.push((key.into(), val.into()));
        self
    }

    /// Clear the environment before applying env vars.
    pub fn env_clear(mut self) -> Self {
        self.env_clear = true;
        self
    }

    /// Set the working directory.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Set the stdin mode.
    pub fn stdin(mut self, io: Io) -> Self {
        self.stdin = io;
        self
    }

    /// Set the stdout mode.
    pub fn stdout(mut self, io: Io) -> Self {
        self.stdout = io;
        self
    }

    /// Set the stderr mode.
    pub fn stderr(mut self, io: Io) -> Self {
        self.stderr = io;
        self
    }

    /// Set the process group configuration.
    pub fn process_group(mut self, pg: ProcessGroup) -> Self {
        self.process_group = pg;
        self
    }
}

impl std::fmt::Debug for SpawnConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnConfig")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env_clear", &self.env_clear)
            .field("cwd", &self.cwd)
            .field("process_group", &self.process_group)
            .finish_non_exhaustive()
    }
}

// ── Child ───────────────────────────────────────────────────────────────

/// A running child process.
///
/// Provides access to the process ID, stdio pipes, and wait/try_wait
/// operations. When dropped, the child is reaped (Unix) or the handle
/// is closed (Windows).
pub struct Child {
    pid: u32,
    inner: ChildInner,
    /// Pipe connected to the child's stdin (write end), if `Io::Pipe` was used.
    pub stdin: Option<std::process::ChildStdin>,
    /// Pipe connected to the child's stdout (read end), if `Io::Pipe` was used.
    pub stdout: Option<std::process::ChildStdout>,
    /// Pipe connected to the child's stderr (read end), if `Io::Pipe` was used.
    pub stderr: Option<std::process::ChildStderr>,
}

impl std::fmt::Debug for Child {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Child")
            .field("pid", &self.pid)
            .field("stdin", &self.stdin.is_some())
            .field("stdout", &self.stdout.is_some())
            .field("stderr", &self.stderr.is_some())
            .finish()
    }
}

// ── ChildInner (platform-specific) ──────────────────────────────────────

#[cfg(unix)]
struct ChildInner {
    pid: i32,
}

#[cfg(unix)]
impl Drop for ChildInner {
    fn drop(&mut self) {
        // Non-blocking reap to prevent zombie processes. If the child has
        // already been reaped by wait()/try_wait(), waitpid returns ECHILD
        // which we silently ignore.
        use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
        let nix_pid = nix::unistd::Pid::from_raw(self.pid);
        match waitpid(nix_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::StillAlive) => {
                tracing::warn!(
                    pid = self.pid,
                    "child process still running when handle dropped; it may become a zombie"
                );
            }
            Ok(_) => {}
            Err(nix::errno::Errno::ECHILD) => {}
            Err(e) => {
                tracing::warn!(
                    pid = self.pid,
                    error = %e,
                    "waitpid failed during drop"
                );
            }
        }
    }
}

#[cfg(windows)]
struct ChildInner {
    handle: ::windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for ChildInner {
    fn drop(&mut self) {
        // SAFETY: self.handle is a valid process handle obtained from
        // DuplicateHandle that we own exclusively.
        unsafe {
            ::windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

// SAFETY: Windows process HANDLEs are not tied to a specific thread.
// WaitForSingleObject, GetExitCodeProcess, TerminateProcess, and
// CloseHandle are all safe to call from any thread.
#[cfg(windows)]
unsafe impl Send for ChildInner {}

#[cfg(not(any(unix, windows)))]
struct ChildInner;

impl Child {
    /// Returns the OS-level process ID.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Block until the child exits, returning its [`ExitStatus`].
    ///
    /// On Unix, signal kills produce 128+N. On Windows, the raw exit code
    /// from `GetExitCodeProcess` is returned.
    pub fn wait(&mut self) -> Result<ExitStatus, SpawnError> {
        #[cfg(unix)]
        {
            unix::wait_blocking(self.inner.pid)
        }
        #[cfg(windows)]
        {
            windows::wait_blocking(self.inner.handle)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(ExitStatus::from_raw(0))
        }
    }

    /// Non-blocking check: returns `Some(ExitStatus)` if the child has exited,
    /// `None` if still running.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SpawnError> {
        #[cfg(unix)]
        {
            unix::try_wait(self.inner.pid)
        }
        #[cfg(windows)]
        {
            windows::try_wait(self.inner.handle)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(None)
        }
    }

    /// Take ownership of the child's stdin pipe, if one was created.
    pub fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.stdin.take()
    }

    /// Take ownership of the child's stdout pipe, if one was created.
    pub fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.stdout.take()
    }

    /// Take ownership of the child's stderr pipe, if one was created.
    pub fn take_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.stderr.take()
    }
}

// ── spawn() ─────────────────────────────────────────────────────────────

/// Spawn a child process using the given configuration.
///
/// This is the primary entry point for creating child processes in MALT.
/// It delegates to platform-specific implementations that use
/// `std::process::Command` with appropriate extensions.
pub fn spawn(config: SpawnConfig) -> Result<Child, SpawnError> {
    #[cfg(unix)]
    return unix::spawn(config);
    #[cfg(windows)]
    return windows::spawn(config);
    #[cfg(not(any(unix, windows)))]
    {
        let _ = config;
        Err(SpawnError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "unsupported platform",
        )))
    }
}
