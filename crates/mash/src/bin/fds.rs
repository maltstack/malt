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

fn parse_bound(value: Option<&String>, default: i32) -> i32 {
    value
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(unix)]
fn is_fd_open(fd: i32) -> bool {
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    // SAFETY: `fcntl(fd, F_GETFD)` is a read-only validity probe.
    unsafe { fcntl(fd, 1) != -1 }
}

#[cfg(windows)]
fn is_fd_open(fd: i32) -> bool {
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
