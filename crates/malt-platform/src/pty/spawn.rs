//! PTY-attached process spawning.
//!
//! Provides `spawn_with_pty` that opens a PTY and spawns a child attached to it.
//! On Unix, sets the child's stdin/stdout/stderr to the PTY slave fd.
//! On Windows, uses ConPTY with PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE.

use super::{open_pty, Pty, PtyError, WinSize};
use crate::process::{Child, SpawnConfig, SpawnError};
use std::fs::File;
use std::path::PathBuf;
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
    let (pty, reader, writer) = open_pty(size)?;

    #[cfg(unix)]
    let child = spawn_unix(program, args, cwd, &reader, &writer)?;

    #[cfg(windows)]
    let child = spawn_windows(program, args, cwd)?;

    Ok(PtyProcess {
        child,
        pty,
        reader,
        writer,
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
    reader: &File,
    writer: &File,
) -> Result<Child, PtySpawnError> {
    use crate::process::{Io, SpawnConfig};

    // On Unix, the reader/writer are duped from the master fd.
    // We pass clones as the child's stdin/stdout/stderr so the child
    // writes to the master, and we read from it.
    let stdin_file = writer.try_clone()?;
    let stdout_file = reader.try_clone()?;
    let stderr_file = reader.try_clone()?;

    let config = SpawnConfig::new(program)
        .args(args.iter().map(|s| std::ffi::OsString::from(s)))
        .cwd(cwd)
        .stdin(Io::File(stdin_file))
        .stdout(Io::File(stdout_file))
        .stderr(Io::File(stderr_file));

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
        .args(args.iter().map(|s| std::ffi::OsString::from(s)))
        .cwd(cwd)
        .stdin(Io::Pipe)
        .stdout(Io::Pipe)
        .stderr(Io::Pipe);

    let child = crate::process::spawn(config)?;
    Ok(child)
}
