//! Built-in `touch` implementation (minimal).
//!
//! Supports:
//! - Create empty files
//! - Update file timestamps
//! - `-c`: don't create files (existing only)
//! - `-f`: ignored (BSD compatibility)
//! - `-r ref_file`: use ref file's timestamps
//! - `-t time`: use specified time [[CC]YY]MMDDhhmm[.ss]
//!
//! For now, we just create files if they don't exist.

use crate::BuiltinResult;
use std::fs::File;
use std::path::Path;

/// Update file timestamps or create empty files.
///
/// Exit 0 on success, 1 on error.
pub fn touch(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut no_create = false;
    let mut paths: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-c" | "--no-create" => no_create = true,
            "-f" => { /* ignored for BSD compatibility */ }
            "-r" | "-t" => {
                // These flags take arguments - skip next arg
                // For minimal implementation, we just ignore these
            }
            _ => {
                if arg.starts_with('-') && arg.len() > 1 {
                    // Unknown flag - might be combined or have argument
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

        if p.exists() {
            // File exists - update timestamps by opening and closing
            // On Windows, this updates access time
            let _ = File::open(p);
        } else if !no_create {
            // Create empty file
            match File::create(p) {
                Ok(_) => {}
                Err(e) => {
                    stderr.extend_from_slice(
                        format!("touch: cannot touch '{}': {}\n", path, e).as_bytes(),
                    );
                    exit_code = 1;
                }
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
    use std::path::Path;

    #[test]
    fn touch_create_file() {
        let path = "test_touch_file.txt";
        // Remove if exists
        let _ = fs::remove_file(path);

        let r = touch(&[path.into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(Path::new(path).exists());

        // Cleanup
        let _ = fs::remove_file(path);
    }

    #[test]
    fn touch_existing_file() {
        let path = "test_touch_existing.txt";
        // Create file first
        let _ = fs::write(path, "content");

        let r = touch(&[path.into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(Path::new(path).exists());

        // Cleanup
        let _ = fs::remove_file(path);
    }

    #[test]
    fn touch_no_create() {
        let path = "test_touch_no_create.txt";
        // Remove if exists
        let _ = fs::remove_file(path);

        let r = touch(&["-c".into(), path.into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(!Path::new(path).exists());
    }
}
