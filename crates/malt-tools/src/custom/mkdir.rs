//! Built-in `mkdir` implementation (minimal).
//!
//! Supports:
//! - `-p`: create parent directories as needed
//! - `-m mode`: set permission mode (ignored on Windows)

use crate::BuiltinResult;
use std::fs;
use std::path::Path;

/// Create directories.
///
/// Exit 0 on success, 1 on error.
pub fn mkdir(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    let mut create_parents = false;
    let mut paths: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--parents" => create_parents = true,
            "-m" => {
                // Skip mode argument
                i += 1;
            }
            _ => {
                if args[i].starts_with('-') {
                    // Unknown flag
                } else {
                    paths.push(&args[i]);
                }
            }
        }
        i += 1;
    }

    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for path in paths {
        let p = Path::new(path);

        if create_parents {
            if let Err(e) = fs::create_dir_all(p) {
                stderr.extend_from_slice(
                    format!("mkdir: cannot create directory '{}': {}\n", path, e).as_bytes(),
                );
                exit_code = 1;
            }
        } else {
            if let Err(e) = fs::create_dir(p) {
                stderr.extend_from_slice(
                    format!("mkdir: cannot create directory '{}': {}\n", path, e).as_bytes(),
                );
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
    fn mkdir_single() {
        let path = "test_mkdir_single";
        // Remove if exists
        let _ = fs::remove_dir(path);

        let r = mkdir(&[path.into()], b"");
        assert_eq!(r.exit_code, 0);
        assert!(Path::new(path).exists());

        // Cleanup
        let _ = fs::remove_dir(path);
    }

    #[test]
    fn mkdir_parents() {
        let path = "test_mkdir_parent/child/grandchild";
        // Remove if exists
        let _ = fs::remove_dir_all("test_mkdir_parent");

        let r = mkdir(&["-p".into(), path.into()], b"");
        assert_eq!(r.exit_code, 0);
        assert!(Path::new(path).exists());

        // Cleanup
        let _ = fs::remove_dir_all("test_mkdir_parent");
    }
}
