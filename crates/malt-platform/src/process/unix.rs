//! Unix process spawning via `std::process::Command` with `pre_exec`.
//!
//! After spawning, we take the pid and stdio pipes from the std `Child`,
//! then `std::mem::forget()` it so its destructor does not call `waitpid`.
//! We manage the process lifecycle ourselves via `nix::sys::wait::waitpid`.

use super::{Child, ChildInner, ExitStatus, Io, ProcessGroup, SpawnConfig, SpawnError};
use std::os::fd::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::os::unix::process::CommandExt;
use std::process::Stdio;

/// Convert an `Io` variant to a `Stdio` for use with `std::process::Command`.
fn io_to_stdio(io: Io) -> Result<Stdio, SpawnError> {
    match io {
        Io::Inherit => Ok(Stdio::inherit()),
        Io::Null => Ok(Stdio::null()),
        Io::Pipe => Ok(Stdio::piped()),
        Io::File(f) => Ok(f.into()),
        Io::Fd(fd) => {
            // SAFETY: The caller guarantees that `fd` is a valid, open file
            // descriptor. We duplicate it so the original remains owned by the
            // caller. The dup'd fd will be consumed by Command.
            let duped = nix::unistd::dup(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) })
                .map_err(|e| SpawnError::Io(std::io::Error::from(e)))?;
            // SAFETY: duped is a valid fd from dup() that we own.
            Ok(unsafe { Stdio::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(duped)) })
        }
    }
}

/// Map a spawn IO error to the appropriate `SpawnError` variant.
fn map_spawn_error(e: std::io::Error, program: &std::path::Path) -> SpawnError {
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

pub(super) fn parent_pid() -> Option<u32> {
    let ppid = nix::unistd::getppid().as_raw();
    (ppid > 0).then_some(ppid as u32)
}

pub(super) fn spawn(config: SpawnConfig) -> Result<Child, SpawnError> {
    let extra_fds = config.extra_fds;
    let close_fds = config.close_fds;
    let mut cmd = std::process::Command::new(&config.program);
    cmd.args(&config.args);

    // POSIX: argv[0] may differ from the program path.
    if let Some(ref a0) = config.argv0 {
        cmd.arg0(a0);
    }

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

    // Resolve process group for the pre_exec closure.
    let pg = match &config.process_group {
        ProcessGroup::Inherit => None,
        ProcessGroup::New => Some(0i32),
        ProcessGroup::Join(pgid) => {
            let pgid_i32 =
                i32::try_from(*pgid).map_err(|_| SpawnError::InvalidPgid { pgid: *pgid })?;
            Some(pgid_i32)
        }
    };

    // SAFETY: This closure runs between fork() and exec() in the child process.
    // We only call async-signal-safe functions here.
    unsafe {
        cmd.pre_exec(move || {
            if let Some(pgid) = pg {
                let result = nix::unistd::setpgid(
                    nix::unistd::Pid::from_raw(0),
                    nix::unistd::Pid::from_raw(pgid),
                );
                if let Err(e) = result {
                    return Err(std::io::Error::from_raw_os_error(e as i32));
                }
            }
            for fd in &close_fds {
                if libc::close(*fd) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            for (target_fd, source_file) in &extra_fds {
                let src_fd = source_file.as_raw_fd();
                if src_fd != *target_fd && libc::dup2(src_fd, *target_fd) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(*target_fd, libc::F_GETFD);
                if flags >= 0 {
                    let _ = libc::fcntl(*target_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().map_err(|e| map_spawn_error(e, &config.program))?;

    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Take ownership of the process via its PID. The std::process::Child is
    // forgotten so its destructor does not call waitpid.
    std::mem::forget(child);

    Ok(Child {
        pid,
        inner: ChildInner { pid: pid as i32 },
        stdin,
        stdout,
        stderr,
    })
}

/// Blocking wait for a child process to exit.
pub(super) fn wait_blocking(pid: i32) -> Result<ExitStatus, SpawnError> {
    use nix::sys::wait::{waitpid, WaitStatus};

    let nix_pid = nix::unistd::Pid::from_raw(pid);
    match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => Ok(ExitStatus::from_raw(code)),
        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(ExitStatus::from_raw(128 + sig as i32)),
        Ok(_) => Ok(ExitStatus::from_raw(-1)),
        Err(e) => Err(SpawnError::Io(std::io::Error::other(e.to_string()))),
    }
}

/// Non-blocking wait: returns `Some` if exited, `None` if still running.
pub(super) fn try_wait(pid: i32) -> Result<Option<ExitStatus>, SpawnError> {
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};

    let nix_pid = nix::unistd::Pid::from_raw(pid);
    match waitpid(nix_pid, Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(_, code)) => Ok(Some(ExitStatus::from_raw(code))),
        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(Some(ExitStatus::from_raw(128 + sig as i32))),
        Ok(WaitStatus::StillAlive) => Ok(None),
        Ok(_) => Ok(None),
        Err(e) => Err(SpawnError::Io(std::io::Error::other(e.to_string()))),
    }
}
