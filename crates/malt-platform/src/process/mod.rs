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

/// Render a Windows command line using the same quoting rules as the native
/// process-spawn path. HCS accepts one command-line string, while MASH owns a
/// program plus argument vector; keeping this conversion in `malt-platform`
/// prevents helper and host spawning from drifting on spaces, quotes, or
/// trailing backslashes.
pub fn windows_command_line(program: &str, arguments: &[String]) -> String {
    let mut command_line = quote_windows_argument(program);
    for argument in arguments {
        command_line.push(' ');
        command_line.push_str(&quote_windows_argument(argument));
    }
    command_line
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_string();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

// ── Stdio handle types ─────────────────────────────────────────────────
//
// On Windows there are two distinct pipe-creation paths with different I/O
// modes: the plain `std::process::Command` path creates genuinely
// overlapped-mode handles (which `std::process::Child{Stdin,Stdout,Stderr}`'s
// `Read`/`Write` impls expect), while the `argv0`-override path
// (`CreateProcessW` + raw `CreatePipe`, used for every `mash`
// external-command spawn) creates synchronous handles. Wrapping a
// synchronous handle as `ChildStdin`/`ChildStdout` and reading or writing it
// more than once hangs indefinitely -- found by actually reading a
// multi-write child's output incrementally, not by inspection. Each variant
// below carries the handle in the concrete type whose I/O mode actually
// matches how it was created, so `Read`/`Write` always dispatch correctly.
// On Unix there is no such split -- a fd is a fd -- so these are plain type
// aliases and every existing call site is unaffected.

#[cfg(windows)]
pub enum ChildStdoutHandle {
    Overlapped(std::process::ChildStdout),
    Sync(std::fs::File),
}
#[cfg(windows)]
pub enum ChildStderrHandle {
    Overlapped(std::process::ChildStderr),
    Sync(std::fs::File),
}
#[cfg(windows)]
pub enum ChildStdinHandle {
    Overlapped(std::process::ChildStdin),
    Sync(std::fs::File),
}

#[cfg(windows)]
impl std::io::Read for ChildStdoutHandle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Overlapped(h) => h.read(buf),
            Self::Sync(h) => h.read(buf),
        }
    }
}
#[cfg(windows)]
impl std::io::Read for ChildStderrHandle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Overlapped(h) => h.read(buf),
            Self::Sync(h) => h.read(buf),
        }
    }
}
#[cfg(windows)]
impl std::io::Write for ChildStdinHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Overlapped(h) => h.write(buf),
            Self::Sync(h) => h.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Overlapped(h) => h.flush(),
            Self::Sync(h) => h.flush(),
        }
    }
}
#[cfg(windows)]
impl std::os::windows::io::IntoRawHandle for ChildStdoutHandle {
    fn into_raw_handle(self) -> std::os::windows::io::RawHandle {
        match self {
            Self::Overlapped(h) => h.into_raw_handle(),
            Self::Sync(h) => h.into_raw_handle(),
        }
    }
}
#[cfg(windows)]
impl std::os::windows::io::IntoRawHandle for ChildStdinHandle {
    fn into_raw_handle(self) -> std::os::windows::io::RawHandle {
        match self {
            Self::Overlapped(h) => h.into_raw_handle(),
            Self::Sync(h) => h.into_raw_handle(),
        }
    }
}

#[cfg(unix)]
pub type ChildStdoutHandle = std::process::ChildStdout;
#[cfg(unix)]
pub type ChildStderrHandle = std::process::ChildStderr;
#[cfg(unix)]
pub type ChildStdinHandle = std::process::ChildStdin;

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
    pub stdin: Option<ChildStdinHandle>,
    /// Pipe connected to the child's stdout (read end), if `Io::Pipe` was used.
    pub stdout: Option<ChildStdoutHandle>,
    /// Pipe connected to the child's stderr (read end), if `Io::Pipe` was used.
    pub stderr: Option<ChildStderrHandle>,
    /// Relay tasks installed by an alternate launcher to preserve requested
    /// file/null/inherited stdio while the child itself exposes HCS pipes.
    io_workers: Vec<std::thread::JoinHandle<std::io::Result<()>>>,
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
enum ChildInner {
    Native {
        handle: ::windows_sys::Win32::Foundation::HANDLE,
    },
    Hcs {
        handle: isize,
    },
}

#[cfg(windows)]
impl Drop for ChildInner {
    fn drop(&mut self) {
        match self {
            Self::Native { handle } => {
                // SAFETY: handle is a valid process handle obtained from
                // DuplicateHandle that we own exclusively.
                unsafe {
                    ::windows_sys::Win32::Foundation::CloseHandle(*handle);
                }
            }
            Self::Hcs { handle } => {
                if let Err(error) = crate::isolation::hcs::close_process_handle(*handle) {
                    tracing::warn!(%error, "failed to close HCS process handle");
                }
            }
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
        let status = { unix::wait_blocking(self.inner.pid) };
        #[cfg(windows)]
        let status = {
            match self.inner {
                ChildInner::Native { handle } => windows::wait_blocking(handle),
                ChildInner::Hcs { handle } => crate::isolation::hcs::wait_process_exit(handle)
                    .map(ExitStatus::from_raw)
                    .map_err(|error| SpawnError::Io(std::io::Error::other(error.to_string()))),
            }
        };
        #[cfg(not(any(unix, windows)))]
        let status = Ok(ExitStatus::from_raw(0));
        let status = status?;
        self.join_io_workers()?;
        Ok(status)
    }

    /// Non-blocking check: returns `Some(ExitStatus)` if the child has exited,
    /// `None` if still running.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SpawnError> {
        #[cfg(unix)]
        let result = { unix::try_wait(self.inner.pid) };
        #[cfg(windows)]
        let result = {
            match self.inner {
                ChildInner::Native { handle } => windows::try_wait(handle),
                ChildInner::Hcs { handle } => crate::isolation::hcs::try_wait_process_exit(handle)
                    .map(|result| result.map(ExitStatus::from_raw))
                    .map_err(|error| SpawnError::Io(std::io::Error::other(error.to_string()))),
            }
        };
        #[cfg(not(any(unix, windows)))]
        let result = Ok(None);
        let result = result?;
        if result.is_some() {
            self.join_io_workers()?;
        }
        Ok(result)
    }

    /// Take ownership of the child's stdin pipe, if one was created.
    pub fn take_stdin(&mut self) -> Option<ChildStdinHandle> {
        self.stdin.take()
    }

    /// Take ownership of the child's stdout pipe, if one was created.
    pub fn take_stdout(&mut self) -> Option<ChildStdoutHandle> {
        self.stdout.take()
    }

    /// Take ownership of the child's stderr pipe, if one was created.
    pub fn take_stderr(&mut self) -> Option<ChildStderrHandle> {
        self.stderr.take()
    }

    /// Attach a stdio relay task installed by an alternate process launcher.
    /// The task is joined after the child exits, so relay failures are never
    /// silently discarded.
    pub fn add_io_worker(&mut self, worker: std::thread::JoinHandle<std::io::Result<()>>) {
        self.io_workers.push(worker);
    }

    fn join_io_workers(&mut self) -> Result<(), SpawnError> {
        for worker in std::mem::take(&mut self.io_workers) {
            match worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(SpawnError::Io(error)),
                Err(_) => {
                    return Err(SpawnError::Io(std::io::Error::other(
                        "child stdio relay task panicked",
                    )))
                }
            }
        }
        Ok(())
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

/// Adopt HCS process and standard-stream handles already duplicated into this
/// process by the privileged helper. The helper authenticates the daemon peer
/// before duplication; this function takes ownership only after that transfer.
#[cfg(windows)]
pub fn child_from_hcs_process(
    process_id: u32,
    process_handle: u64,
    stdin_handle: u64,
    stdout_handle: u64,
    stderr_handle: u64,
) -> Result<Child, SpawnError> {
    windows::child_from_hcs_process(
        process_id,
        process_handle,
        stdin_handle,
        stdout_handle,
        stderr_handle,
    )
}

/// Return the current process's parent PID when the platform can provide it.
pub fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        unix::parent_pid()
    }
    #[cfg(windows)]
    {
        windows::parent_pid()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
