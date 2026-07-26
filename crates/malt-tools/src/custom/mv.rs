//! Built-in `mv` implementation (minimal).
//!
//! Supports:
//! - Move/rename files and directories
//! - `-f`: force overwrite
//! - `-i`: interactive (ignored)

use crate::BuiltinResult;
use std::fs;
use std::path::Path;

/// Move (rename) files.
///
/// Exit 0 on success, 1 on error.
pub fn mv(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut force = false;
    let mut paths: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
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

    if paths.len() < 2 {
        return BuiltinResult::failure(1, "mv: missing file operand\n".into());
    }

    // Last argument is destination
    let dest = paths.pop().unwrap();
    let sources: Vec<_> = paths;

    let dest_path = Path::new(dest);

    // Handle single source move
    if sources.len() == 1 {
        let src = Path::new(sources[0]);

        if !src.exists() {
            return BuiltinResult::failure(
                1,
                format!(
                    "mv: cannot stat '{}': No such file or directory\n",
                    sources[0]
                )
                .into(),
            );
        }

        // Check if destination exists and we're not forcing
        if dest_path.exists() && !force {
            return BuiltinResult::failure(
                1,
                format!("mv: cannot move to '{}': File exists\n", dest).into(),
            );
        }

        match fs::rename(src, dest_path) {
            Ok(_) => BuiltinResult::success(Vec::new()),
            Err(e) => BuiltinResult::failure(
                1,
                format!("mv: cannot move '{}': {}\n", sources[0], e).into(),
            ),
        }
    } else {
        // Multiple sources - destination must be directory
        if !dest_path.exists() {
            return BuiltinResult::failure(
                1,
                format!("mv: target directory '{}' does not exist\n", dest).into(),
            );
        }
        if !dest_path.is_dir() {
            return BuiltinResult::failure(
                1,
                format!("mv: target '{}' is not a directory\n", dest).into(),
            );
        }

        let mut errors = Vec::new();
        let mut exit_code = 0;

        for src in sources {
            let src_path = Path::new(src);
            if !src_path.exists() {
                errors.extend_from_slice(
                    format!("mv: cannot stat '{}': No such file or directory\n", src).as_bytes(),
                );
                exit_code = 1;
                continue;
            }

            let file_name = src_path.file_name().unwrap_or_default();
            let dest_file = dest_path.join(file_name);

            if dest_file.exists() && !force {
                errors.extend_from_slice(
                    format!("mv: cannot move '{}': File exists at destination\n", src).as_bytes(),
                );
                exit_code = 1;
                continue;
            }

            if let Err(e) = fs::rename(src_path, dest_file) {
                errors.extend_from_slice(format!("mv: cannot move '{}': {}\n", src, e).as_bytes());
                exit_code = 1;
            }
        }

        BuiltinResult {
            exit_code,
            stdout: Vec::new(),
            stderr: errors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn mv_rename_file() {
        let src = "test_mv_src.txt";
        let dst = "test_mv_dst.txt";

        fs::write(src, "content").unwrap();
        let _ = fs::remove_file(dst);

        let r = mv(
            &[src.into(), dst.into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert!(!Path::new(src).exists());
        assert!(Path::new(dst).exists());

        // Cleanup
        let _ = fs::remove_file(dst);
    }

    #[test]
    fn mv_nonexistent() {
        let src = "test_mv_nonexistent.txt";
        let dst = "test_mv_dest.txt";
        let _ = fs::remove_file(src);
        let _ = fs::remove_file(dst);

        let r = mv(
            &[src.into(), dst.into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 1);
        assert!(!Path::new(dst).exists());
    }
}
