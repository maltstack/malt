fn main() {
    #[cfg(windows)]
    // SAFETY: installs a process-local no-op CRT invalid-parameter handler
    // before probing file descriptors so invalid probes return errors.
    unsafe {
        install_noop_invalid_parameter_handler();
    }

    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(|s| s.as_str()) == Some("--fd-state") {
        let state_path = args.get(2).expect("--fd-state requires a path argument");
        let start = parse_bound(args.get(3), 0);
        let end = parse_bound(args.get(4), 9);
        run_with_file_state(state_path, start, end);
    } else {
        let start = parse_bound(args.get(1), 0);
        let end = parse_bound(args.get(2), 9);
        run_with_env_state(start, end);
    }
}

fn run_with_file_state(state_path: &str, start: i32, end: i32) {
    let fd_state = FdState::from_file(state_path);
    for fd in start..=end {
        let state = if fd_state.is_declared(fd) || is_fd_open(fd) {
            "open"
        } else {
            "closed"
        };
        println!("{fd} {state}");
    }
}

fn run_with_env_state(start: i32, end: i32) {
    for fd in start..=end {
        let state = if shell_managed_fd_is_open(fd) || is_fd_open(fd) {
            "open"
        } else {
            "closed"
        };
        println!("{fd} {state}");
    }
}

struct FdState {
    declared_fds: Vec<i32>,
}

impl FdState {
    fn from_file(path: &str) -> Self {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let mut declared_fds = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // File format: comma-separated entries, each "fd:target" or "fd|path"
            for entry in line.split(',').filter(|e| !e.is_empty()) {
                let entry = entry.trim();
                if let Some((fd_text, _)) = entry.split_once(':') {
                    if let Ok(fd) = fd_text.parse() {
                        declared_fds.push(fd);
                    }
                } else if let Some((fd_text, _)) = entry.split_once('|') {
                    if let Ok(fd) = fd_text.parse() {
                        declared_fds.push(fd);
                    }
                }
            }
        }
        Self { declared_fds }
    }

    fn is_declared(&self, fd: i32) -> bool {
        self.declared_fds.contains(&fd)
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

    // SAFETY: `_dup` returns `-1` for invalid descriptors.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_temp_state_file(aliases: &str, snapshots: &str) -> std::path::PathBuf {
        let file = std::env::temp_dir();
        let mut path;
        let mut attempts = 0;
        loop {
            let name = format!(
                "mash_fd_test_{}_{}_{}",
                std::process::id(),
                std::time::Instant::now().elapsed().as_nanos(),
                attempts
            );
            path = file.join(name);
            // Try to create new file, fail if exists
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let mut content = String::new();
                    if !aliases.is_empty() {
                        content.push_str(aliases);
                        content.push('\n');
                    }
                    if !snapshots.is_empty() {
                        content.push_str(snapshots);
                        content.push('\n');
                    }
                    f.write_all(content.as_bytes()).unwrap();
                    break;
                }
                Err(_) => {
                    attempts += 1;
                    if attempts > 100 {
                        panic!("Could not create unique temp file after 100 attempts");
                    }
                }
            }
        }
        path
    }

    #[test]
    fn alias_file_marks_declared_fd_open() {
        let path = create_temp_state_file("3:1,4:2", "");
        let state = FdState::from_file(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(state.is_declared(3));
        assert!(state.is_declared(4));
        assert!(!state.is_declared(5));
    }

    #[test]
    fn snapshot_file_marks_declared_fd_open() {
        let path = create_temp_state_file("", "5|616263,6|646566");
        let state = FdState::from_file(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(state.is_declared(5));
        assert!(state.is_declared(6));
        assert!(!state.is_declared(7));
    }

    #[test]
    fn empty_file_declares_no_fds() {
        let path = create_temp_state_file("", "");
        let state = FdState::from_file(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(!state.is_declared(3));
        assert!(!state.is_declared(5));
    }
}
