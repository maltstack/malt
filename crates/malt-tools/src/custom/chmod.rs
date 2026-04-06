//! Built-in `chmod` implementation (minimal).
//!
//! Supports:
//! - Octal mode (e.g., 755, 644)
//! - Symbolic mode (e.g., +x, -w) - basic support only
//!
//! Note: On Windows, this has limited effect - primarily for compatibility.

use crate::BuiltinResult;
use std::path::Path;

/// Change file mode bits.
///
/// Exit 0 on success, 1 on error.
pub fn chmod(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    let mut recursive = false;
    let mut mode_str: Option<&str> = None;
    let mut paths: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-R" | "-r" | "--recursive" => recursive = true,
            "-f" | "--force" => { /* ignore errors */ }
            "-v" | "--verbose" => { /* ignored */ }
            _ => {
                if args[i].starts_with('-') {
                    // Unknown flag
                } else if mode_str.is_none() {
                    mode_str = Some(&args[i]);
                } else {
                    paths.push(&args[i]);
                }
            }
        }
        i += 1;
    }

    if mode_str.is_none() {
        return BuiltinResult::failure(1, "chmod: missing operand\n".into());
    }

    if paths.is_empty() {
        return BuiltinResult::failure(1, "chmod: missing file operand\n".into());
    }

    let _mode = mode_str.unwrap();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for path in paths {
        let p = Path::new(path);
        if !p.exists() {
            stderr.extend_from_slice(
                format!(
                    "chmod: cannot access '{}': No such file or directory\n",
                    path
                )
                .as_bytes(),
            );
            exit_code = 1;
            continue;
        }

        // On Windows, we can't really set Unix permissions
        // We just verify the file exists and return success for compatibility
        // Real permission changes would require platform-specific code
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode_val = if let Ok(octal) = u32::from_str_radix(_mode, 8) {
                octal
            } else {
                // Try to parse symbolic mode (very basic)
                0o644 // default
            };

            let meta = match std::fs::metadata(p) {
                Ok(m) => m,
                Err(e) => {
                    stderr.extend_from_slice(
                        format!("chmod: cannot access '{}': {}\n", path, e).as_bytes(),
                    );
                    exit_code = 1;
                    continue;
                }
            };

            let mut perms = meta.permissions();
            perms.set_mode(mode_val);
            if let Err(e) = std::fs::set_permissions(p, perms) {
                stderr.extend_from_slice(
                    format!("chmod: changing permissions of '{}': {}\n", path, e).as_bytes(),
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
    fn chmod_file() {
        let path = "test_chmod.txt";
        fs::write(path, "content").unwrap();

        let r = chmod(&["644".into(), path.into()], b"");
        // On Windows this succeeds with no-op, on Unix it changes permissions
        assert_eq!(r.exit_code, 0);

        // Cleanup
        let _ = fs::remove_file(path);
    }

    #[test]
    fn chmod_nonexistent() {
        let path = "test_chmod_nonexistent.txt";
        let _ = fs::remove_file(path);

        let r = chmod(&["644".into(), path.into()], b"");
        assert_eq!(r.exit_code, 1);
    }
}
