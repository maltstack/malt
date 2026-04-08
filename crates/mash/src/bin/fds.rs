fn main() {
    #[cfg(windows)]
    // SAFETY: installs a process-local no-op CRT invalid-parameter handler
    // before probing file descriptors so invalid probes return errors.
    unsafe {
        install_noop_invalid_parameter_handler();
    }

    let args: Vec<String> = std::env::args().collect();
    let start = parse_bound(args.get(1), 0);
    let end = parse_bound(args.get(2), 9);

    for fd in start..=end {
        let state = if is_fd_open(fd) { "open" } else { "closed" };
        println!("{fd} {state}");
    }
}

const MASH_FD_ALIASES_ENV: &str = "MASH_FD_ALIASES";
const MASH_FD_SNAPSHOTS_ENV: &str = "MASH_FD_SNAPSHOTS";

fn parse_bound(value: Option<&String>, default: i32) -> i32 {
    value
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(unix)]
fn is_fd_open(fd: i32) -> bool {
    if shell_managed_fd_is_open(fd) {
        return true;
    }
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    // SAFETY: `fcntl(fd, F_GETFD)` is a read-only validity probe.
    unsafe { fcntl(fd, 1) != -1 }
}

#[cfg(windows)]
fn is_fd_open(fd: i32) -> bool {
    if shell_managed_fd_is_open(fd) {
        return true;
    }
    unsafe extern "C" {
        fn _close(fd: i32) -> i32;
        fn _dup(fd: i32) -> i32;
    }

    // SAFETY: after installing the no-op invalid-parameter handler above,
    // `_dup` returns `-1` for invalid descriptors instead of terminating.
    let duplicated = unsafe { _dup(fd) };
    if duplicated == -1 {
        return false;
    }

    // SAFETY: `duplicated` is a valid CRT file descriptor produced by `_dup`.
    unsafe {
        let _ = _close(duplicated);
    }
    true
}

fn shell_managed_fd_is_open(fd: i32) -> bool {
    env_declares_fd(fd, MASH_FD_ALIASES_ENV, ':') || env_declares_fd(fd, MASH_FD_SNAPSHOTS_ENV, '|')
}

fn env_declares_fd(fd: i32, env_name: &str, separator: char) -> bool {
    let Ok(spec) = std::env::var(env_name) else {
        return false;
    };
    spec.split(',')
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| entry.split_once(separator))
        .filter_map(|(fd_text, _)| fd_text.parse::<i32>().ok())
        .any(|declared_fd| declared_fd == fd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_env_marks_declared_fd_open() {
        unsafe {
            std::env::set_var(MASH_FD_ALIASES_ENV, "3:1,4:2");
        }
        assert!(shell_managed_fd_is_open(3));
        assert!(shell_managed_fd_is_open(4));
        assert!(!shell_managed_fd_is_open(5));
        unsafe {
            std::env::remove_var(MASH_FD_ALIASES_ENV);
        }
    }

    #[test]
    fn snapshot_env_marks_declared_fd_open() {
        unsafe {
            std::env::set_var(MASH_FD_SNAPSHOTS_ENV, "5|616263,6|646566");
        }
        assert!(shell_managed_fd_is_open(5));
        assert!(shell_managed_fd_is_open(6));
        assert!(!shell_managed_fd_is_open(7));
        unsafe {
            std::env::remove_var(MASH_FD_SNAPSHOTS_ENV);
        }
    }
}

#[cfg(windows)]
unsafe fn install_noop_invalid_parameter_handler() {
    unsafe extern "C" fn noop(
        _expression: *const u16,
        _function: *const u16,
        _file: *const u16,
        _line: u32,
        _reserved: usize,
    ) {
    }

    unsafe extern "C" {
        fn _set_invalid_parameter_handler(
            handler: unsafe extern "C" fn(*const u16, *const u16, *const u16, u32, usize),
        ) -> unsafe extern "C" fn(*const u16, *const u16, *const u16, u32, usize);
    }

    // SAFETY: installs a handler with the CRT's required signature.
    unsafe {
        let _ = _set_invalid_parameter_handler(noop);
    }
}
