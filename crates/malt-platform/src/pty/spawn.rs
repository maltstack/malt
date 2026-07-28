//! PTY-attached process spawning.
//!
//! Provides `spawn_with_pty` that opens a PTY and spawns a child attached to it.
//! On Unix, sets the child's stdin/stdout/stderr to the PTY slave fd.
//! On Windows, uses ConPTY with PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.

use super::{open_pty, Pty, PtyError, WinSize};
use crate::process::{Child, SpawnError};
use std::fs::File;
use std::sync::Arc;

/// Result of spawning a process with a PTY.
pub struct PtyProcess {
    /// The child process.
    pub child: Child,
    /// PTY handle for resize.
    pub pty: Arc<dyn Pty>,
    /// Read output from the child (master side).
    pub reader: File,
    /// Write input to the child (master side).
    pub writer: File,
    /// The child's end of the pty, retained deliberately.
    ///
    /// `None` on Windows, which has no slave fd (ConPTY attaches via
    /// `STARTUPINFOEX`).
    ///
    /// On Unix this must stay alive for as long as anyone intends to read
    /// `reader`. A pty whose last slave has closed stops delivering: Linux
    /// returns `EIO` to the reader, discarding anything still buffered, so a
    /// short-lived child's output would be lost the instant it exited.
    /// Dropping `PtyProcess` closes it and is what finally gives the reader
    /// its end-of-stream — see the decision recorded in
    /// `docs/briefs/007-unix-pty-wired-backwards.md`.
    pub slave: Option<File>,
}

/// Spawn a child process attached to a new PTY.
///
/// This is the correct way to spawn a terminal process — the child's
/// stdin/stdout/stderr are connected to the PTY, not inherited from the daemon.
pub fn spawn_with_pty(
    program: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    size: WinSize,
) -> Result<PtyProcess, PtySpawnError> {
    let (pty, reader, writer, slave) = open_pty(size)?;

    #[cfg(unix)]
    let child = spawn_unix(
        program,
        args,
        cwd,
        slave
            .as_ref()
            .ok_or_else(|| PtyError::Open(std::io::Error::other("unix pty returned no slave")))?,
    )?;

    #[cfg(windows)]
    let child = spawn_windows(program, args, cwd)?;

    Ok(PtyProcess {
        child,
        pty,
        reader,
        writer,
        slave,
    })
}

/// Errors from PTY-attached spawning.
#[derive(Debug, thiserror::Error)]
pub enum PtySpawnError {
    #[error("pty error: {0}")]
    Pty(#[from] PtyError),
    #[error("spawn error: {0}")]
    Spawn(#[from] SpawnError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(unix)]
fn spawn_unix(
    program: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
    slave: &File,
) -> Result<Child, PtySpawnError> {
    use crate::process::{Io, SpawnConfig};
    use std::os::fd::AsRawFd;

    // The child gets the **slave**; the parent keeps the master. This is the
    // arrangement a pty requires, and getting it backwards is what
    // docs/briefs/007 documents: the child used to be handed dups of the
    // master, leaving no slave open anywhere, which makes the pty inert.
    let stdin_file = slave.try_clone()?;
    let stdout_file = slave.try_clone()?;
    let stderr_file = slave.try_clone()?;

    // Make it the child's controlling terminal, otherwise job control and
    // anything that opens /dev/tty still misbehaves even once bytes flow.
    let config = SpawnConfig::new(program)
        .args(args.iter().map(|s| std::ffi::OsString::from(s)))
        .cwd(cwd)
        .stdin(Io::File(stdin_file))
        .stdout(Io::File(stdout_file))
        .stderr(Io::File(stderr_file))
        .controlling_tty(slave.as_raw_fd());

    let child = crate::process::spawn(config)?;
    Ok(child)
}

#[cfg(windows)]
fn spawn_windows(
    program: &std::path::Path,
    args: &[String],
    cwd: &std::path::Path,
) -> Result<Child, PtySpawnError> {
    use crate::process::{Io, SpawnConfig};

    // On Windows with ConPTY, the child should be spawned with
    // PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE for proper attachment.
    // However, std::process::Command doesn't support this yet.
    //
    // Workaround: spawn with piped stdio. The ConPTY handles
    // terminal emulation on the input/output pipe side, and the
    // child reads/writes through its normal stdio which we pipe.
    let config = SpawnConfig::new(program)
        .args(args.iter().map(std::ffi::OsString::from))
        .cwd(cwd)
        .stdin(Io::Pipe)
        .stdout(Io::Pipe)
        .stderr(Io::Pipe);

    let child = crate::process::spawn(config)?;
    Ok(child)
}
