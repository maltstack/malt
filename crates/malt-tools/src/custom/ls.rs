//! Built-in `ls` implementation (minimal).
//!
//! Supports basic POSIX ls flags used by Smoosh tests:
//! - `-a`: show hidden files
//! - `-l`: long listing format
//! - `-d`: list directories themselves

use crate::BuiltinResult;
use std::path::Path;

/// List directory contents.
///
/// Exit 0 on success, 1 on error.
pub fn ls(args: &[String], _stdin: &mut dyn std::io::Read) -> BuiltinResult {
    let mut show_all = false;
    let mut long_format = false;
    let mut dir_itself = false;
    let mut paths: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-a" | "-A" | "--all" => show_all = true,
            "-l" | "--format=long" => long_format = true,
            "-la" | "-al" => {
                show_all = true;
                long_format = true;
            }
            "-d" | "--directory" => dir_itself = true,
            _ => {
                if arg.starts_with('-') {
                    // Unknown flag - ignore for now
                } else {
                    paths.push(arg);
                }
            }
        }
    }

    if paths.is_empty() {
        paths.push(".");
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for path in paths {
        let p = Path::new(path);
        if !p.exists() {
            stderr.extend_from_slice(
                format!("ls: cannot access '{}': No such file or directory\n", path).as_bytes(),
            );
            exit_code = 1;
            continue;
        }

        if p.is_file() || (dir_itself && p.is_dir()) {
            // Single file or directory itself
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string());
            if long_format {
                let meta = match std::fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => {
                        stderr.extend_from_slice(
                            format!("ls: cannot access '{}': {}\n", path, e).as_bytes(),
                        );
                        exit_code = 1;
                        continue;
                    }
                };
                let size = meta.len();
                let perms = if meta.is_dir() {
                    "drwxr-xr-x"
                } else {
                    "-rw-r--r--"
                };
                let line = format!("{} 1 user user {:>8} Jan  1 00:00 {}\n", perms, size, name);
                stdout.extend_from_slice(line.as_bytes());
            } else {
                stdout.extend_from_slice(format!("{}\n", name).as_bytes());
            }
        } else if p.is_dir() && !dir_itself {
            // List directory contents
            let entries = match std::fs::read_dir(p) {
                Ok(e) => e,
                Err(e) => {
                    stderr.extend_from_slice(
                        format!("ls: cannot open directory '{}': {}\n", path, e).as_bytes(),
                    );
                    exit_code = 1;
                    continue;
                }
            };

            let mut names: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !show_all && name.starts_with('.') {
                    continue;
                }
                names.push(name);
            }
            names.sort_unstable();

            for name in names {
                if long_format {
                    let full_path = p.join(&name);
                    let meta = match std::fs::metadata(&full_path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let size = meta.len();
                    let perms = if meta.is_dir() {
                        "drwxr-xr-x"
                    } else {
                        "-rw-r--r--"
                    };
                    let line = format!("{} 1 user user {:>8} Jan  1 00:00 {}\n", perms, size, name);
                    stdout.extend_from_slice(line.as_bytes());
                } else {
                    stdout.extend_from_slice(format!("{}\n", name).as_bytes());
                }
            }
        }
    }

    BuiltinResult {
        exit_code,
        stdout,
        stderr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_current_dir() {
        let r = ls(&[], &mut &b""[..]);
        assert_eq!(r.exit_code, 0);
        assert!(!r.stdout.is_empty());
    }

    #[test]
    fn ls_nonexistent() {
        let r = ls(&["/nonexistent".into()], &mut &b""[..]);
        assert_eq!(r.exit_code, 1);
    }
}
