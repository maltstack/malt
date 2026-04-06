//! Built-in `rm` implementation (minimal).
//!
//! Supports:
//! - `-r`: recursive removal
//! - `-f`: force (ignore errors, no prompt)
//! - `-i`: interactive (ignored for now)

use crate::BuiltinResult;
use std::fs;
use std::path::Path;

/// Remove files or directories.
///
/// Exit 0 on success, 1 on error (unless -f).
pub fn rm(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    let mut recursive = false;
    let mut force = false;
    let mut paths: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-r" | "-R" | "--recursive" => recursive = true,
            "-f" | "--force" => force = true,
            "-i" | "--interactive" => { /* ignored */ }
            _ => {
                if arg.starts_with('-') && arg.len() > 1 {
                    // Unknown flag
                } else {
                    paths.push(arg);
                }
            }
        }
    }

    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for path in paths {
        let p = Path::new(path);

        if !p.exists() {
            if !force {
                stderr.extend_from_slice(
                    format!("rm: cannot remove '{}': No such file or directory\n", path).as_bytes(),
                );
                exit_code = 1;
            }
            continue;
        }

        let result = if p.is_dir() {
            if recursive {
                fs::remove_dir_all(p)
            } else {
                if !force {
                    stderr.extend_from_slice(
                        format!("rm: cannot remove '{}': Is a directory\n", path).as_bytes(),
                    );
                    exit_code = 1;
                }
                continue;
            }
        } else {
            fs::remove_file(p)
        };

        if let Err(e) = result {
            if !force {
                stderr
                    .extend_from_slice(format!("rm: cannot remove '{}': {}\n", path, e).as_bytes());
                exit_code = 1;
            }
        }
    }

    BuiltinResult {
        exit_code,
        stdout: Vec::new(),
        stderr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rm_file() {
        let path = "test_rm_file.txt";
        fs::write(path, "content").unwrap();

        let r = rm(&[path.into()], b"");
        assert_eq!(r.exit_code, 0);
        assert!(!Path::new(path).exists());
    }

    #[test]
    fn rm_nonexistent() {
        let path = "test_rm_nonexistent.txt";
        // Ensure doesn't exist
        let _ = fs::remove_file(path);

        let r = rm(&[path.into()], b"");
        assert_eq!(r.exit_code, 1);
    }

    #[test]
    fn rm_force() {
        let path = "test_rm_force.txt";
        // Ensure doesn't exist
        let _ = fs::remove_file(path);

        let r = rm(&["-f".into(), path.into()], b"");
        assert_eq!(r.exit_code, 0);
    }
}
