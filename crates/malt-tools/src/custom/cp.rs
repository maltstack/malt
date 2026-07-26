//! Built-in `cp` implementation (minimal).
//!
//! Supports:
//! - Copy files
//! - `-r`: recursive copy for directories
//! - `-f`: force overwrite

use crate::BuiltinResult;
use std::fs;
use std::io;
use std::path::Path;

/// Copy files and directories.
///
/// Exit 0 on success, 1 on error.
pub fn cp(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut recursive = false;
    let mut force = false;
    let mut paths: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-r" | "-R" | "--recursive" => recursive = true,
            "-f" | "--force" => force = true,
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
        return BuiltinResult::failure(1, "cp: missing file operand\n".into());
    }

    let dest = paths.pop().unwrap();
    let sources: Vec<_> = paths;
    let dest_path = Path::new(dest);

    // Handle single source copy
    if sources.len() == 1 {
        let src = Path::new(sources[0]);

        if !src.exists() {
            return BuiltinResult::failure(
                1,
                format!(
                    "cp: cannot stat '{}': No such file or directory\n",
                    sources[0]
                )
                .into(),
            );
        }

        if src.is_file() {
            // Check if destination exists and we're not forcing
            if dest_path.exists() && !force {
                return BuiltinResult::failure(
                    1,
                    format!("cp: cannot copy to '{}': File exists\n", dest).into(),
                );
            }

            match fs::copy(src, dest_path) {
                Ok(_) => BuiltinResult::success(Vec::new()),
                Err(e) => BuiltinResult::failure(
                    1,
                    format!("cp: cannot copy '{}': {}\n", sources[0], e).into(),
                ),
            }
        } else if src.is_dir() {
            if !recursive {
                return BuiltinResult::failure(
                    1,
                    format!(
                        "cp: -r not specified; omitting directory '{}'\n",
                        sources[0]
                    )
                    .into(),
                );
            }
            // Recursive copy would go here - simplified for now
            match copy_dir(src, dest_path, force) {
                Ok(_) => BuiltinResult::success(Vec::new()),
                Err(e) => BuiltinResult::failure(
                    1,
                    format!("cp: cannot copy directory '{}': {}\n", sources[0], e).into(),
                ),
            }
        } else {
            BuiltinResult::failure(
                1,
                format!("cp: cannot copy '{}': unknown file type\n", sources[0]).into(),
            )
        }
    } else {
        // Multiple sources - destination must be directory
        if !dest_path.exists() {
            return BuiltinResult::failure(
                1,
                format!("cp: target directory '{}' does not exist\n", dest).into(),
            );
        }
        if !dest_path.is_dir() {
            return BuiltinResult::failure(
                1,
                format!("cp: target '{}' is not a directory\n", dest).into(),
            );
        }

        let mut errors = Vec::new();
        let mut exit_code = 0;

        for src in sources {
            let src_path = Path::new(src);
            if !src_path.exists() {
                errors.extend_from_slice(
                    format!("cp: cannot stat '{}': No such file or directory\n", src).as_bytes(),
                );
                exit_code = 1;
                continue;
            }

            if src_path.is_dir() && !recursive {
                errors.extend_from_slice(
                    format!("cp: -r not specified; omitting directory '{}'\n", src).as_bytes(),
                );
                exit_code = 1;
                continue;
            }

            let file_name = src_path.file_name().unwrap_or_default();
            let dest_file = dest_path.join(file_name);

            if dest_file.exists() && !force {
                errors.extend_from_slice(
                    format!("cp: cannot copy '{}': File exists at destination\n", src).as_bytes(),
                );
                exit_code = 1;
                continue;
            }

            let copy_result: io::Result<()> = if src_path.is_file() {
                fs::copy(src_path, &dest_file).map(|_| ())
            } else {
                copy_dir(src_path, &dest_file, force)
            };

            if let Err(e) = copy_result {
                errors.extend_from_slice(format!("cp: cannot copy '{}': {}\n", src, e).as_bytes());
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

/// Simple recursive directory copy
fn copy_dir(src: &Path, dst: &Path, force: bool) -> io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let dest_path = dst.join(&name);

        if path.is_file() {
            if dest_path.exists() && !force {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file exists"));
            }
            fs::copy(&path, dest_path)?;
        } else if path.is_dir() {
            copy_dir(&path, &dest_path, force)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn cp_file() {
        let src = "test_cp_src.txt";
        let dst = "test_cp_dst.txt";

        fs::write(src, "content").unwrap();
        let _ = fs::remove_file(dst);

        let r = cp(
            &[src.into(), dst.into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert!(Path::new(dst).exists());

        // Cleanup
        let _ = fs::remove_file(src);
        let _ = fs::remove_file(dst);
    }

    #[test]
    fn cp_nonexistent() {
        let src = "test_cp_nonexistent.txt";
        let dst = "test_cp_dest.txt";
        let _ = fs::remove_file(src);
        let _ = fs::remove_file(dst);

        let r = cp(
            &[src.into(), dst.into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 1);
    }
}
