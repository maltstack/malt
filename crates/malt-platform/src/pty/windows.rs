//! Windows ConPTY implementation via `CreatePseudoConsole`.

use super::{Pty, PtyError, WinSize};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, S_OK};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::Pipes::CreatePipe;

/// ConPTY-based pseudo console for Windows 10 1809+.
///
/// Wraps an `HPCON` handle obtained from `CreatePseudoConsole`.
/// Only stores the pseudo console handle itself — pipe handles for
/// reader/writer are returned to the caller from [`open_pty`].
/// The `HPCON` is closed on drop via `ClosePseudoConsole`.
struct ConPty {
    hpc: HPCON,
}

// SAFETY: HPCON is an opaque OS handle. The Windows documentation for
// CreatePseudoConsole / ResizePseudoConsole / ClosePseudoConsole does not
// require thread affinity — these APIs are safe to call from any thread.
// ConPty owns the HPCON exclusively and no aliased mutable access occurs.
unsafe impl Send for ConPty {}
unsafe impl Sync for ConPty {}

impl Pty for ConPty {
    fn resize(&self, size: WinSize) -> Result<(), PtyError> {
        let coord = COORD {
            X: size.cols as i16,
            Y: size.rows as i16,
        };
        // SAFETY: self.hpc is a valid HPCON obtained from CreatePseudoConsole.
        // ResizePseudoConsole accepts any valid HPCON and a COORD value.
        let hr = unsafe { ResizePseudoConsole(self.hpc, coord) };
        if hr != S_OK {
            Err(PtyError::ConPty { code: hr as u32 })
        } else {
            Ok(())
        }
    }
}

impl Drop for ConPty {
    fn drop(&mut self) {
        // SAFETY: self.hpc is a valid HPCON that we own exclusively.
        // ClosePseudoConsole is the documented cleanup function.
        unsafe {
            ClosePseudoConsole(self.hpc);
        }
    }
}

/// Open a new Windows ConPTY pseudo console.
///
/// Returns `(pty_handle, reader, writer)`:
/// - `pty_handle` — owns the HPCON for resize/close
/// - `reader` — reads output from the child process (output pipe read end)
/// - `writer` — writes input to the child process (input pipe write end)
///
/// The ConPTY internally duplicates the pipe endpoints it needs, so after
/// `CreatePseudoConsole` we close `input_read` and `output_write` (our copies)
/// and hand back `output_read` and `input_write` as the reader/writer.
pub(super) fn open_pty(size: WinSize) -> Result<super::OpenedPty, PtyError> {
    use std::os::windows::io::FromRawHandle;

    let coord = COORD {
        X: size.cols as i16,
        Y: size.rows as i16,
    };

    let mut input_read: HANDLE = INVALID_HANDLE_VALUE;
    let mut input_write: HANDLE = INVALID_HANDLE_VALUE;
    let mut output_read: HANDLE = INVALID_HANDLE_VALUE;
    let mut output_write: HANDLE = INVALID_HANDLE_VALUE;

    // SAFETY: CreatePipe is called with valid pointers to HANDLE variables.
    // A null SECURITY_ATTRIBUTES pointer means default security and
    // non-inheritable handles. Buffer size 0 uses the system default.
    unsafe {
        if CreatePipe(&mut input_read, &mut input_write, std::ptr::null(), 0) == 0 {
            return Err(PtyError::Open(std::io::Error::last_os_error()));
        }
        if CreatePipe(&mut output_read, &mut output_write, std::ptr::null(), 0) == 0 {
            CloseHandle(input_read);
            CloseHandle(input_write);
            return Err(PtyError::Open(std::io::Error::last_os_error()));
        }
    }

    let mut hpc: HPCON = 0;

    // SAFETY: CreatePseudoConsole is called with valid pipe handles from
    // CreatePipe. `input_read` is the read end of the input pipe (ConPTY
    // reads from it) and `output_write` is the write end of the output
    // pipe (ConPTY writes to it). The resulting HPCON is stored in ConPty
    // and closed in its Drop impl.
    let hr = unsafe { CreatePseudoConsole(coord, input_read, output_write, 0, &mut hpc) };

    // ConPTY has internally duplicated input_read and output_write; close
    // our copies since we no longer need them.
    // SAFETY: input_read and output_write are valid handles from CreatePipe.
    unsafe {
        CloseHandle(input_read);
        CloseHandle(output_write);
    }

    if hr != S_OK {
        // SAFETY: Clean up the remaining pipe handles on failure.
        unsafe {
            CloseHandle(input_write);
            CloseHandle(output_read);
        }
        return Err(PtyError::ConPty { code: hr as u32 });
    }

    // SAFETY: output_read is a valid HANDLE from CreatePipe that has not been
    // closed. File takes ownership and will close it on drop.
    let reader = unsafe { std::fs::File::from_raw_handle(output_read) };
    // SAFETY: input_write is a valid HANDLE from CreatePipe that has not been
    // closed. File takes ownership and will close it on drop.
    let writer = unsafe { std::fs::File::from_raw_handle(input_write) };

    let pty_handle = Arc::new(ConPty { hpc });
    // `None`: ConPTY has no slave fd to hand a child. The pseudoconsole is
    // attached via STARTUPINFOEX instead, so there is nothing for the caller
    // to keep alive. The Option exists for the Unix arm.
    Ok((pty_handle, reader, writer, None))
}
