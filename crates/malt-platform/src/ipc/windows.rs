use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// The OS identity attached to a connected named-pipe client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerIdentity {
    /// Windows process identifier supplied by the kernel for this connection.
    pub process_id: u32,
}

/// A connected named pipe. The `File` owns and closes the native handle.
#[derive(Debug)]
pub struct NamedPipeConnection {
    file: File,
}

impl NamedPipeConnection {
    /// Return the process identity Windows associates with the connected client.
    pub fn peer_identity(&self) -> io::Result<PeerIdentity> {
        let mut process_id = 0u32;
        // SAFETY: `file` owns a valid connected named-pipe handle for this
        // connection, and `process_id` is a valid writable `u32` pointer.
        let ok = unsafe {
            GetNamedPipeClientProcessId(self.file.as_raw_handle() as *mut _, &mut process_id)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PeerIdentity { process_id })
    }

    /// Borrow the connection as a standard file for framed I/O.
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }
}

/// One named-pipe server instance, ready to accept exactly one client.
#[derive(Debug)]
pub struct NamedPipeServer {
    file: File,
}

impl NamedPipeServer {
    /// Create a local-only duplex named pipe at `\\\\.\\pipe\\<name>`.
    pub fn create(name: &str) -> io::Result<Self> {
        let path = pipe_path(name)?;
        // SAFETY: `path` is NUL terminated and remains live for the duration
        // of the call; all scalar flags and buffer sizes are Win32 constants.
        let handle = unsafe {
            CreateNamedPipeW(
                path.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful `CreateNamedPipeW` transfers ownership of this
        // handle to the returned `File`, which closes it exactly once.
        let file = unsafe { File::from_raw_handle(handle as *mut _) };
        Ok(Self { file })
    }

    /// Wait for one client and return a connection that owns the pipe handle.
    pub fn accept(self) -> io::Result<NamedPipeConnection> {
        // SAFETY: `file` owns a valid server named-pipe handle; null requests
        // synchronous operation, matching the pipe's non-overlapped creation.
        let connected =
            unsafe { ConnectNamedPipe(self.file.as_raw_handle() as *mut _, std::ptr::null_mut()) };
        if connected == 0 {
            // A client can connect between CreateNamedPipeW and this call;
            // Windows reports that race as ERROR_PIPE_CONNECTED.
            // SAFETY: GetLastError has no parameters and reads thread-local OS state.
            let error = unsafe { GetLastError() };
            if error != ERROR_PIPE_CONNECTED {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
        }
        Ok(NamedPipeConnection { file: self.file })
    }
}

/// Client connection factory for an existing helper named pipe.
pub struct NamedPipeClient;

impl NamedPipeClient {
    /// Connect to an existing local named pipe.
    pub fn connect(name: &str) -> io::Result<NamedPipeConnection> {
        let path = pipe_path(name)?;
        // SAFETY: `path` is NUL terminated and remains live for the duration
        // of the call. The remaining parameters are documented Win32 values.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful CreateFileW transfers ownership of this handle
        // to the returned File, which will close it exactly once.
        Ok(NamedPipeConnection {
            file: unsafe { File::from_raw_handle(handle as *mut _) },
        })
    }
}

fn pipe_path(name: &str) -> io::Result<Vec<u16>> {
    if name.is_empty() || name.contains('\\') || name.contains('/') || name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid named-pipe name",
        ));
    }
    Ok(format!(r"\\.\pipe\{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}
