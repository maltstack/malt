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
pub fn rm(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut recursive = false;
    let mut force = false;
    let mut paths: Vec<&str> = Vec::new();
    let mut options_ended = false;

    for arg in args {
        if options_ended {
            paths.push(arg);
            continue;
        }

        if arg == "--" {
            options_ended = true;
            continue;
        }

        match arg.as_str() {
            "-r" | "-R" | "--recursive" => recursive = true,
            "-f" | "--force" => force = true,
            "-i" | "--interactive" => { /* ignored */ }
            _ => {
                if let Some(flags) = arg.strip_prefix('-') {
                    if !flags.is_empty() {
                        for flag in flags.chars() {
                            match flag {
                                'r' | 'R' => recursive = true,
                                'f' => force = true,
                                'i' => {}
                                _ => {
                                    return BuiltinResult {
                                        exit_code: 1,
                                        stdout: Vec::new(),
                                        stderr: format!("rm: invalid option -- '{}'\n", flag)
                                            .into_bytes(),
                                    };
                                }
                            }
                        }
                        continue;
                    }
                }
                paths.push(arg);
            }
        }
    }

    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for path in paths {
        let p = Path::new(path);
        let metadata = match fs::symlink_metadata(p) {
            Ok(metadata) => metadata,
            Err(_) => {
                if !force {
                    stderr.extend_from_slice(
                        format!("rm: cannot remove '{}': No such file or directory\n", path)
                            .as_bytes(),
                    );
                    exit_code = 1;
                }
                continue;
            }
        };

        let result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
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

        let r = rm(&[path.into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(!Path::new(path).exists());
    }

    #[test]
    fn rm_nonexistent() {
        let path = "test_rm_nonexistent.txt";
        // Ensure doesn't exist
        let _ = fs::remove_file(path);

        let r = rm(&[path.into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 1);
    }

    #[test]
    fn rm_force() {
        let path = "test_rm_force.txt";
        // Ensure doesn't exist
        let _ = fs::remove_file(path);

        let r = rm(
            &["-f".into(), path.into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
    }

    #[test]
    fn rm_combined_recursive_force_flags_remove_directory() {
        let path = Path::new("test_rm_combined_flags");
        let nested = path.join("nested.txt");
        let _ = fs::remove_dir_all(path);
        fs::create_dir_all(path).unwrap();
        fs::write(&nested, "content").unwrap();

        let r = rm(
            &["-rf".into(), path.display().to_string()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(
            r.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&r.stderr)
        );
        assert!(!path.exists());
    }

    #[test]
    fn rm_rejects_unknown_option_without_removing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.txt");
        fs::write(&path, "content").unwrap();

        let result = rm(
            &["-z".into(), path.display().to_string()],
            &mut &b""[..],
            &mut std::io::sink(),
        );

        assert_eq!(result.exit_code, 1);
        assert!(String::from_utf8_lossy(&result.stderr).contains("rm: invalid option"));
        assert!(String::from_utf8_lossy(&result.stderr).contains("'z'"));
        assert!(path.exists());
    }

    #[test]
    fn rm_rejects_unknown_option_in_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target.txt");
        fs::write(&path, "content").unwrap();

        let result = rm(
            &["-fz".into(), path.display().to_string()],
            &mut &b""[..],
            &mut std::io::sink(),
        );

        assert_eq!(result.exit_code, 1);
        assert!(String::from_utf8_lossy(&result.stderr).contains("'z'"));
        assert!(path.exists());
    }

    #[test]
    fn rm_removes_symlink_without_following_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, "content").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        #[cfg(windows)]
        {
            if let Err(err) = malt_platform::fs::create_symlink(&target, &link) {
                let stderr = err.to_string();
                assert!(
                    stderr.contains("1314") || stderr.contains("A required privilege is not held"),
                    "stderr: {stderr}"
                );
                return;
            }
        }

        fs::remove_file(&target).unwrap();

        let result = rm(
            &[link.display().to_string()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!link.exists());
        assert!(fs::symlink_metadata(&link).is_err());
    }
}
