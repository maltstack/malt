//! Unix PTY implementation via `nix::pty::openpty`.

use super::{Pty, PtyError, WinSize};
use nix::pty::{openpty, OpenptyResult};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

/// Unix PTY handle wrapping the master file descriptor.
///
/// Implements [`Pty`] for resize via `TIOCSWINSZ` ioctl.
/// The master fd is closed on drop via `OwnedFd`.
struct UnixPty {
    master_fd: OwnedFd,
}

impl Pty for UnixPty {
    fn resize(&self, size: WinSize) -> Result<(), PtyError> {
        let ws = nix::pty::Winsize {
            ws_col: size.cols,
            ws_row: size.rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: self.master_fd is a valid open PTY master file descriptor
        // obtained from openpty. TIOCSWINSZ is the correct ioctl for setting
        // terminal window size, and &ws points to a valid, properly initialized
        // Winsize struct that outlives the ioctl call.
        let ret =
            unsafe { nix::libc::ioctl(self.master_fd.as_raw_fd(), nix::libc::TIOCSWINSZ, &ws) };
        if ret == -1 {
            Err(PtyError::Resize(std::io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }
}

/// Set `FD_CLOEXEC` on a file descriptor to prevent it from leaking
/// into child processes across exec boundaries.
fn set_cloexec(fd: &OwnedFd) -> Result<(), PtyError> {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).map_err(|e| PtyError::Open(e.into()))?;
    Ok(())
}

/// Open a new Unix PTY pair.
///
/// Returns `(pty_handle, reader, writer)`:
/// - `pty_handle` retains the original master fd for resize
/// - `reader` and `writer` are independent dup'd handles to the master fd
/// - The slave fd is dropped here; it will be passed to child processes
///   via `SpawnConfig` separately.
pub(super) fn open_pty(
    size: WinSize,
) -> Result<(Arc<dyn Pty>, std::fs::File, std::fs::File), PtyError> {
    let winsize = Some(nix::pty::Winsize {
        ws_col: size.cols,
        ws_row: size.rows,
        ws_xpixel: 0,
        ws_ypixel: 0,
    });

    let OpenptyResult { master, slave } =
        openpty(&winsize, None).map_err(|e| PtyError::Open(e.into()))?;

    // Set O_CLOEXEC on both fds to prevent leaking into child processes
    // on exec. openpty(3) does not guarantee this flag is set.
    set_cloexec(&master)?;
    set_cloexec(&slave)?;

    // Dup master fd for independent reader and writer ownership.
    let reader_fd: OwnedFd = nix::unistd::dup(&master)
        .map_err(|e| PtyError::Open(std::io::Error::from_raw_os_error(e as i32)))?;
    let writer_fd: OwnedFd = nix::unistd::dup(&master)
        .map_err(|e| PtyError::Open(std::io::Error::from_raw_os_error(e as i32)))?;

    let reader = std::fs::File::from(reader_fd);
    let writer = std::fs::File::from(writer_fd);

    // Drop slave fd — it will be passed to child process via SpawnConfig.
    drop(slave);

    let pty_handle = Arc::new(UnixPty { master_fd: master });
    Ok((pty_handle, reader, writer))
}
