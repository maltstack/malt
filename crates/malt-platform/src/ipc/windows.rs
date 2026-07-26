use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::mpsc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, FILETIME, GENERIC_READ,
    GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::System::IO::CancelSynchronousIo;

const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

/// The OS identity attached to a connected named-pipe client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerIdentity {
    /// Windows process identifier supplied by the kernel for this connection.
    pub process_id: u32,
    /// Canonical Windows user SID read from that process's access token.
    pub principal: String,
}

/// Kernel-observed identity evidence for a process enrolled with the helper.
///
/// `creation_time_100ns` prevents PID reuse from being mistaken for an
/// already-authorised daemon. The service compares every field it relies on
/// again at operation time rather than trusting a caller-supplied PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub process_id: u32,
    pub principal: String,
    pub creation_time_100ns: u64,
    pub image_path: String,
    pub elevated: bool,
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
        Ok(PeerIdentity {
            process_id,
            principal: principal_for_process(process_id)?,
        })
    }

    /// Borrow the connection as a standard file for framed I/O.
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }

    /// Read one VNP frame within a hard connection deadline.
    ///
    /// The framing reader uses synchronous ReadFile through File. Windows lets
    /// another thread cancel that pending operation when it owns a
    /// THREAD_TERMINATE-capable handle, keeping a partial frame from occupying
    /// a helper client slot forever.
    pub fn read_frame_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<malt_protocol::framing::Frame, malt_protocol::framing::FrameError> {
        use malt_protocol::framing::{FrameError, FrameReader};

        let mut reader_file = self.file.try_clone().map_err(FrameError::Io)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::Builder::new()
            .name("malt-named-pipe-frame-read".to_string())
            .spawn(move || {
                let result = FrameReader::new(&mut reader_file).read_frame();
                let _ = sender.send(result);
            })
            .map_err(FrameError::Io)?;
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                reader.join().map_err(|_| {
                    FrameError::Io(io::Error::other(
                        "named-pipe frame reader panicked after completing I/O",
                    ))
                })?;
                result
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // SAFETY: reader is a live Rust thread. Its native handle is
                // valid until JoinHandle is consumed, and the reader issues
                // only synchronous I/O on its cloned pipe handle.
                let cancelled = unsafe { CancelSynchronousIo(reader.as_raw_handle() as *mut _) };
                if cancelled == 0 {
                    let error = io::Error::last_os_error();
                    // ERROR_NOT_FOUND means the frame read completed in the
                    // race between the deadline and cancellation. Joining is
                    // then immediate and the deadline still wins.
                    if error.raw_os_error() != Some(1168) {
                        return Err(FrameError::Io(error));
                    }
                }
                reader.join().map_err(|_| {
                    FrameError::Io(io::Error::other(
                        "named-pipe frame reader panicked while cancelling I/O",
                    ))
                })?;
                Err(FrameError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "named-pipe frame read exceeded its deadline",
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = reader.join();
                Err(FrameError::Io(io::Error::other(
                    "named-pipe frame reader exited without reporting a result",
                )))
            }
        }
    }
}

/// Return the Windows user SID of the current process token.
pub fn current_process_principal() -> io::Result<String> {
    // SAFETY: GetCurrentProcess returns the documented pseudo-handle for this
    // process, which `principal_for_handle` uses only to open its token.
    principal_for_handle(unsafe { GetCurrentProcess() })
}

/// Inspect a process using kernel and token information, not caller input.
pub fn process_identity(process_id: u32) -> io::Result<ProcessIdentity> {
    // SAFETY: process_id is an opaque value supplied by the caller; the
    // requested access permits inspection only and cannot modify the process.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let result = process_identity_for_handle(process, process_id);
    // SAFETY: process is a successful OpenProcess handle owned here.
    if unsafe { CloseHandle(process) } == 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

fn principal_for_process(process_id: u32) -> io::Result<String> {
    // SAFETY: the process ID was supplied by the kernel for this named-pipe
    // connection; the requested access is limited to token inspection.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let result = principal_for_handle(process);
    // SAFETY: `process` is a successful OpenProcess handle owned here.
    if unsafe { CloseHandle(process) } == 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

fn process_identity_for_handle(
    process: windows_sys::Win32::Foundation::HANDLE,
    process_id: u32,
) -> io::Result<ProcessIdentity> {
    let principal = principal_for_handle(process)?;
    let creation_time_100ns = process_creation_time(process)?;
    let image_path = process_image_path(process)?;
    let elevated = process_is_elevated(process)?;
    Ok(ProcessIdentity {
        process_id,
        principal,
        creation_time_100ns,
        image_path,
        elevated,
    })
}

fn process_creation_time(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: process is a valid query handle and each FILETIME points to
    // writable storage for the documented process timestamp outputs.
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(u64::from(created.dwLowDateTime) | (u64::from(created.dwHighDateTime) << 32))
}

fn process_image_path(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    const MAX_IMAGE_PATH_CHARS: usize = 32_768;
    let mut image = vec![0u16; MAX_IMAGE_PATH_CHARS];
    let mut length = image.len() as u32;
    // SAFETY: process is a valid query handle, image has capacity for length
    // UTF-16 characters, and length points to writable size storage.
    if unsafe { QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length) } == 0 {
        return Err(io::Error::last_os_error());
    }
    String::from_utf16(&image[..length as usize])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid process image path"))
}

fn process_is_elevated(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<bool> {
    let mut token = std::ptr::null_mut();
    // SAFETY: process is a valid query handle and token is writable storage.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut elevation = windows_sys::Win32::Security::TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY: token is valid, elevation is writable and exactly the size
    // required by TokenElevation, and returned is writable size storage.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut windows_sys::Win32::Security::TOKEN_ELEVATION).cast(),
            std::mem::size_of::<windows_sys::Win32::Security::TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    let query_error = (queried == 0).then(io::Error::last_os_error);
    // SAFETY: token is a successful OpenProcessToken handle owned here.
    if unsafe { CloseHandle(token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if let Some(error) = query_error {
        return Err(error);
    }
    Ok(elevation.TokenIsElevated != 0)
}

fn principal_for_handle(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    let mut token = std::ptr::null_mut();
    // SAFETY: `process` is a valid process or pseudo-process handle; `token`
    // points to writable storage; TOKEN_QUERY cannot alter the token.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = principal_from_token(token);
    // SAFETY: `token` is a successful OpenProcessToken handle owned here.
    if unsafe { CloseHandle(token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

fn principal_from_token(token: windows_sys::Win32::Foundation::HANDLE) -> io::Result<String> {
    let mut required = 0u32;
    // SAFETY: this documented sizing call writes only `required`; a null
    // buffer with length zero requests the needed TOKEN_USER buffer length.
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0u8; required as usize];
    // SAFETY: `bytes` is allocated to the size the preceding Win32 call
    // reported, and `token` remains valid for the call.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            bytes.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `bytes` holds a complete TOKEN_USER returned by Win32, so its
    // leading representation is valid for the lifetime of `bytes`.
    let user = unsafe { &*(bytes.as_ptr().cast::<TOKEN_USER>()) };
    let mut sid_text = std::ptr::null_mut();
    // SAFETY: `user.User.Sid` is supplied in the validated TOKEN_USER buffer;
    // Windows allocates the returned NUL-terminated string for LocalFree.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `sid_text` is a valid NUL-terminated wide string allocated by
    // Windows; scanning stops at its terminator before it is released.
    let length = unsafe {
        let mut value = 0usize;
        while *sid_text.add(value) != 0 {
            value += 1;
        }
        value
    };
    // SAFETY: the preceding scan established `length` initialized UTF-16
    // elements at `sid_text` before its NUL terminator.
    let principal = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid user SID from Windows"));
    // SAFETY: `sid_text` is the allocation returned by ConvertSidToStringSidW
    // and must be released with LocalFree exactly once.
    if unsafe { LocalFree(sid_text.cast()) }.is_null() {
        principal
    } else {
        Err(io::Error::last_os_error())
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
        create_server(name, std::ptr::null())
    }

    /// Create a local named pipe that grants read/write access only to the
    /// given user SID, local administrators, and LocalSystem.
    pub fn create_for_principal(name: &str, principal: &str) -> io::Result<Self> {
        let security = PipeSecurity::for_principal(principal)?;
        create_server(name, security.attributes())
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

fn create_server(name: &str, security: *const SECURITY_ATTRIBUTES) -> io::Result<NamedPipeServer> {
    let path = pipe_path(name)?;
    // SAFETY: `path` is NUL terminated and remains live for the duration
    // of the call; all scalar flags and buffer sizes are Win32 constants.
    let handle = unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            security,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `CreateNamedPipeW` transfers ownership of this
    // handle to the returned `File`, which closes it exactly once.
    let file = unsafe { File::from_raw_handle(handle as *mut _) };
    Ok(NamedPipeServer { file })
}

struct PipeSecurity {
    descriptor: *mut core::ffi::c_void,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    fn for_principal(principal: &str) -> io::Result<Self> {
        if !principal.starts_with("S-")
            || principal
                .bytes()
                .any(|byte| !(byte.is_ascii_digit() || byte == b'-' || byte == b'S'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "principal must be a canonical Windows SID",
            ));
        }
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{principal})");
        let sddl = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL terminated and valid for this call; Windows
        // allocates the descriptor returned through `descriptor` for LocalFree.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn attributes(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: `descriptor` is allocated by
        // ConvertStringSecurityDescriptorToSecurityDescriptorW and owned here.
        let result = unsafe { LocalFree(self.descriptor) };
        debug_assert!(
            result.is_null(),
            "LocalFree(pipe security descriptor) failed"
        );
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
