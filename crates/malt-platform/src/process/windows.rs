//! Windows process spawning.
//!
//! Uses `std::process::Command` for the common path. When `argv0` override is
//! requested, falls back to a `CreateProcessW` path so PATH-looked-up commands
//! can preserve the shell token in `argv[0]`.

use super::{Child, ChildInner, ExitStatus, Io, ProcessGroup, SpawnConfig, SpawnError};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::process::Stdio;
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, GetExitCodeProcess, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOW,
};

pub(super) fn parent_pid() -> Option<u32> {
    None
}

const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

enum StdioSpec {
    Inherit,
    Null,
    Child(HANDLE),
}

struct ParentPipes {
    // `std::fs::File`, not `std::process::Child{Stdin,Stdout,Stderr}`: these
    // handles come from a plain (non-overlapped) `CreatePipe`, and `File`'s
    // `Read`/`Write` impls issue plain synchronous `ReadFile`/`WriteFile`
    // calls that match that. The `ChildStd*` types assume the overlapped
    // I/O mode that `std::process::Command`'s own pipe creation uses --
    // wrapping a synchronous handle in one of them works for a single
    // `read_to_end`-style call (there happens to be no genuine wait
    // involved) but hangs indefinitely on a second read that must actually
    // block for more data. See the doc comment on `process::Child::stdin`.
    stdin: Option<std::fs::File>,
    stdout: Option<std::fs::File>,
    stderr: Option<std::fs::File>,
}

impl ParentPipes {
    fn empty() -> Self {
        Self {
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }
}

fn io_to_stdio(io: Io) -> Result<Stdio, SpawnError> {
    match io {
        Io::Inherit => Ok(Stdio::inherit()),
        Io::Null => Ok(Stdio::null()),
        Io::Pipe => Ok(Stdio::piped()),
        Io::File(f) => Ok(f.into()),
        Io::Handle(h) => {
            let new_handle = dup_handle(h, false)?;
            // SAFETY: `new_handle` is a valid owned duplicate.
            Ok(unsafe { Stdio::from_raw_handle(new_handle as RawHandle) })
        }
    }
}

fn dup_handle(h: HANDLE, inheritable: bool) -> Result<HANDLE, SpawnError> {
    let mut new_handle: HANDLE = std::ptr::null_mut();
    // SAFETY: pseudo-handle is always valid; DuplicateHandle returns a new owned handle.
    unsafe {
        let current = GetCurrentProcess();
        if DuplicateHandle(
            current,
            h,
            current,
            &mut new_handle,
            0,
            inheritable as i32,
            DUPLICATE_SAME_ACCESS,
        ) == 0
        {
            return Err(SpawnError::Io(std::io::Error::last_os_error()));
        }
    }
    Ok(new_handle)
}

fn map_spawn_error(e: std::io::Error, program: &Path) -> SpawnError {
    match e.kind() {
        std::io::ErrorKind::NotFound => SpawnError::NotFound {
            path: program.to_path_buf(),
        },
        std::io::ErrorKind::PermissionDenied => SpawnError::PermissionDenied {
            path: program.to_path_buf(),
        },
        _ => SpawnError::Io(e),
    }
}

pub(super) fn spawn(config: SpawnConfig) -> Result<Child, SpawnError> {
    if config.argv0.is_some() {
        return spawn_with_create_process(config);
    }

    use std::os::windows::process::CommandExt;

    let mut cmd = std::process::Command::new(&config.program);
    cmd.args(&config.args);

    if config.env_clear {
        cmd.env_clear();
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &config.cwd {
        cmd.current_dir(cwd);
    }

    cmd.stdin(io_to_stdio(config.stdin)?);
    cmd.stdout(io_to_stdio(config.stdout)?);
    cmd.stderr(io_to_stdio(config.stderr)?);

    if matches!(config.process_group, ProcessGroup::New) {
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| map_spawn_error(e, &config.program))?;

    let pid = child.id();
    let stdin = child.stdin.take().map(super::ChildStdinHandle::Overlapped);
    let stdout = child
        .stdout
        .take()
        .map(super::ChildStdoutHandle::Overlapped);
    let stderr = child
        .stderr
        .take()
        .map(super::ChildStderrHandle::Overlapped);
    let handle = child.as_raw_handle() as HANDLE;
    let owned_handle = dup_handle(handle, false)?;
    std::mem::forget(child);

    Ok(Child {
        pid,
        inner: ChildInner::Native {
            handle: owned_handle,
        },
        stdin,
        stdout,
        stderr,
        io_workers: Vec::new(),
    })
}

pub(super) fn child_from_hcs_process(
    process_id: u32,
    process_handle: u64,
    stdin_handle: u64,
    stdout_handle: u64,
    stderr_handle: u64,
) -> Result<Child, SpawnError> {
    let process_handle = checked_handle(process_handle, "HCS process")?;
    let stdin_handle = checked_handle(stdin_handle, "HCS stdin")?;
    let stdout_handle = checked_handle(stdout_handle, "HCS stdout")?;
    let stderr_handle = checked_handle(stderr_handle, "HCS stderr")?;
    // SAFETY: each handle was duplicated into this daemon by the authenticated
    // elevated helper. The new OwnedHandle/File owners close them exactly once.
    let stdin = unsafe { std::fs::File::from_raw_handle(stdin_handle as RawHandle) };
    // SAFETY: see the ownership statement for stdin above.
    let stdout = unsafe { std::fs::File::from_raw_handle(stdout_handle as RawHandle) };
    // SAFETY: see the ownership statement for stdin above.
    let stderr = unsafe { std::fs::File::from_raw_handle(stderr_handle as RawHandle) };
    Ok(Child {
        pid: process_id,
        inner: ChildInner::Native {
            handle: process_handle as RawHandle,
        },
        stdin: Some(super::ChildStdinHandle::Sync(stdin)),
        stdout: Some(super::ChildStdoutHandle::Sync(stdout)),
        stderr: Some(super::ChildStderrHandle::Sync(stderr)),
        io_workers: Vec::new(),
    })
}

fn checked_handle(value: u64, name: &str) -> Result<HANDLE, SpawnError> {
    if value == 0 || value > isize::MAX as u64 {
        return Err(SpawnError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name} handle is invalid"),
        )));
    }
    Ok(value as HANDLE)
}

fn spawn_with_create_process(config: SpawnConfig) -> Result<Child, SpawnError> {
    let mut env_block = build_env_block(&config)?;
    let current_dir = config
        .cwd
        .as_ref()
        .map(|cwd| os_to_wide_null(cwd.as_os_str()));
    let app_name = os_to_wide_null(config.program.as_os_str());
    let argv0 = config
        .argv0
        .clone()
        .unwrap_or_else(|| config.program.as_os_str().to_os_string());
    let mut command_line = build_command_line(&argv0, &config.args);

    let mut parent_pipes = ParentPipes::empty();
    let stdin_spec = prepare_stdio(config.stdin, PipeDirection::Stdin, &mut parent_pipes)?;
    let stdout_spec = prepare_stdio(config.stdout, PipeDirection::Stdout, &mut parent_pipes)?;
    let stderr_spec = prepare_stdio(config.stderr, PipeDirection::Stderr, &mut parent_pipes)?;

    let mut startup_info: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup_info.dwFlags = STARTF_USESTDHANDLES;
    startup_info.hStdInput = stdio_handle(&stdin_spec);
    startup_info.hStdOutput = stdio_handle(&stdout_spec);
    startup_info.hStdError = stdio_handle(&stderr_spec);

    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let mut flags = CREATE_UNICODE_ENVIRONMENT;
    if matches!(config.process_group, ProcessGroup::New) {
        flags |= CREATE_NEW_PROCESS_GROUP;
    }

    // SAFETY: all pointers reference live buffers for the duration of the call,
    // std handles are valid inheritable handles, and PROCESS_INFORMATION /
    // STARTUPINFOW are properly initialized.
    let created = unsafe {
        CreateProcessW(
            app_name.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            flags,
            env_block.as_mut_ptr().cast(),
            current_dir
                .as_ref()
                .map_or(std::ptr::null(), |cwd| cwd.as_ptr()),
            &startup_info,
            &mut process_info,
        )
    };

    close_child_handle(stdin_spec);
    close_child_handle(stdout_spec);
    close_child_handle(stderr_spec);

    if created == 0 {
        return Err(map_spawn_error(
            std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32),
            &config.program,
        ));
    }

    // SAFETY: thread handle is owned by us after CreateProcessW success.
    unsafe {
        CloseHandle(process_info.hThread);
    }

    Ok(Child {
        pid: process_info.dwProcessId,
        inner: ChildInner::Native {
            handle: process_info.hProcess,
        },
        stdin: parent_pipes.stdin.map(super::ChildStdinHandle::Sync),
        stdout: parent_pipes.stdout.map(super::ChildStdoutHandle::Sync),
        stderr: parent_pipes.stderr.map(super::ChildStderrHandle::Sync),
        io_workers: Vec::new(),
    })
}

#[derive(Clone, Copy)]
enum PipeDirection {
    Stdin,
    Stdout,
    Stderr,
}

fn prepare_stdio(
    io: Io,
    direction: PipeDirection,
    parent_pipes: &mut ParentPipes,
) -> Result<StdioSpec, SpawnError> {
    match io {
        Io::Inherit => Ok(StdioSpec::Inherit),
        Io::Null => Ok(StdioSpec::Null),
        Io::File(file) => Ok(StdioSpec::Child(dup_handle(
            file.as_raw_handle() as HANDLE,
            true,
        )?)),
        Io::Handle(handle) => Ok(StdioSpec::Child(dup_handle(handle, true)?)),
        Io::Pipe => create_pipe_stdio(direction, parent_pipes),
    }
}

fn create_pipe_stdio(
    direction: PipeDirection,
    parent_pipes: &mut ParentPipes,
) -> Result<StdioSpec, SpawnError> {
    let mut read_handle: HANDLE = std::ptr::null_mut();
    let mut write_handle: HANDLE = std::ptr::null_mut();
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };

    // SAFETY: outputs are valid pointers and SECURITY_ATTRIBUTES enables inheritance.
    let ok = unsafe { CreatePipe(&mut read_handle, &mut write_handle, &sa, 0) };
    if ok == 0 {
        return Err(SpawnError::Io(std::io::Error::last_os_error()));
    }

    match direction {
        PipeDirection::Stdin => {
            set_non_inheritable(write_handle)?;
            let parent = child_stdin_from_handle(write_handle);
            parent_pipes.stdin = Some(parent);
            Ok(StdioSpec::Child(read_handle))
        }
        PipeDirection::Stdout => {
            set_non_inheritable(read_handle)?;
            let parent = child_stdout_from_handle(read_handle);
            parent_pipes.stdout = Some(parent);
            Ok(StdioSpec::Child(write_handle))
        }
        PipeDirection::Stderr => {
            set_non_inheritable(read_handle)?;
            let parent = child_stderr_from_handle(read_handle);
            parent_pipes.stderr = Some(parent);
            Ok(StdioSpec::Child(write_handle))
        }
    }
}

fn set_non_inheritable(handle: HANDLE) -> Result<(), SpawnError> {
    // SAFETY: handle is valid and owned by this process.
    let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    if ok == 0 {
        return Err(SpawnError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn child_stdin_from_handle(handle: HANDLE) -> std::fs::File {
    // SAFETY: handle is a valid owned write-end of a pipe.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
    owned.into()
}

fn child_stdout_from_handle(handle: HANDLE) -> std::fs::File {
    // SAFETY: handle is a valid owned read-end of a pipe.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
    owned.into()
}

fn child_stderr_from_handle(handle: HANDLE) -> std::fs::File {
    // SAFETY: handle is a valid owned read-end of a pipe.
    let owned = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
    owned.into()
}

fn stdio_handle(spec: &StdioSpec) -> HANDLE {
    match spec {
        StdioSpec::Inherit => INVALID_HANDLE_VALUE,
        StdioSpec::Null => std::ptr::null_mut(),
        StdioSpec::Child(handle) => *handle,
    }
}

fn close_child_handle(spec: StdioSpec) {
    if let StdioSpec::Child(handle) = spec {
        // SAFETY: handle is owned by this process after duplication/CreatePipe.
        unsafe {
            CloseHandle(handle);
        }
    }
}

fn os_to_wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(iter::once(0)).collect()
}

fn build_command_line(argv0: &OsString, args: &[OsString]) -> Vec<u16> {
    let mut line = quote_windows_arg(argv0);
    for arg in args {
        line.push(' ');
        line.push_str(&quote_windows_arg(arg));
    }
    OsStr::new(&line)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn quote_windows_arg(arg: &OsStr) -> String {
    let s = arg.to_string_lossy();
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.into_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in s.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn build_env_block(config: &SpawnConfig) -> Result<Vec<u16>, SpawnError> {
    let mut merged = if config.env_clear {
        BTreeMap::<OsString, OsString>::new()
    } else {
        std::env::vars_os().collect::<BTreeMap<_, _>>()
    };

    for (k, v) in &config.env {
        merged.insert(k.clone(), v.clone());
    }

    let mut block = Vec::new();
    for (key, value) in merged {
        let mut entry = OsString::new();
        entry.push(key);
        entry.push("=");
        entry.push(value);
        block.extend(entry.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

pub(super) fn wait_blocking(handle: HANDLE) -> Result<ExitStatus, SpawnError> {
    // SAFETY: handle is a valid process handle.
    unsafe {
        WaitForSingleObject(handle, INFINITE);
        let mut code: u32 = 0;
        if GetExitCodeProcess(handle, &mut code) == 0 {
            return Err(SpawnError::Io(std::io::Error::last_os_error()));
        }
        Ok(ExitStatus::from_raw(code as i32))
    }
}

pub(super) fn try_wait(handle: HANDLE) -> Result<Option<ExitStatus>, SpawnError> {
    // SAFETY: handle is a valid process handle.
    unsafe {
        let result = WaitForSingleObject(handle, 0);
        if result == WAIT_OBJECT_0 {
            let mut code: u32 = 0;
            if GetExitCodeProcess(handle, &mut code) == 0 {
                return Err(SpawnError::Io(std::io::Error::last_os_error()));
            }
            Ok(Some(ExitStatus::from_raw(code as i32)))
        } else {
            Ok(None)
        }
    }
}
