//! Built-in `ln` implementation.
//!
//! Supports:
//! - hard links
//! - `-s`: symbolic links
//! - `-f`: remove destination before linking

use crate::BuiltinResult;
use std::fs;
use std::io;
use std::path::Path;

pub fn ln(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut symbolic = false;
    let mut force = false;
    let mut operands: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-s" => symbolic = true,
            "-f" => force = true,
            "--" => {}
            _ if arg.starts_with('-') && arg.len() > 1 => {
                return BuiltinResult::failure(
                    1,
                    format!("ln: unsupported option '{}'\n", arg).into(),
                );
            }
            _ => operands.push(arg),
        }
    }

    if operands.len() < 2 {
        return BuiltinResult::failure(1, b"ln: missing file operand\n".to_vec());
    }

    let destination = Path::new(operands.last().copied().unwrap_or_default());
    let sources = &operands[..operands.len() - 1];
    let destination_is_dir = sources.len() > 1 || destination.is_dir();

    if sources.len() > 1 && !destination_is_dir {
        return BuiltinResult::failure(
            1,
            format!(
                "ln: target '{}' is not a directory\n",
                destination.display()
            )
            .into(),
        );
    }

    let mut stderr = Vec::new();
    let mut exit_code = 0;

    for source in sources {
        let link_path = if destination_is_dir {
            match Path::new(source).file_name() {
                Some(name) => destination.join(name),
                None => {
                    stderr
                        .extend_from_slice(format!("ln: invalid source '{}'\n", source).as_bytes());
                    exit_code = 1;
                    continue;
                }
            }
        } else {
            destination.to_path_buf()
        };

        if let Err(err) = create_link(source, &link_path, symbolic, force) {
            stderr.extend_from_slice(
                format!(
                    "ln: failed to link '{}' -> '{}': {}\n",
                    link_path.display(),
                    source,
                    err
                )
                .as_bytes(),
            );
            exit_code = 1;
        }
    }

    BuiltinResult {
        exit_code,
        stdout: Vec::new(),
        stderr,
    }
}

fn create_link(source: &str, link_path: &Path, symbolic: bool, force: bool) -> io::Result<()> {
    if force && link_path.symlink_metadata().is_ok() {
        remove_existing_path(link_path)?;
    }

    if symbolic {
        create_symlink(source, link_path)
    } else {
        fs::hard_link(source, link_path)
    }
}

fn remove_existing_path(path: &Path) -> io::Result<()> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(windows)]
fn create_symlink(target: &str, link_path: &Path) -> io::Result<()> {
    malt_platform::fs::create_symlink(Path::new(target), link_path)
}

#[cfg(unix)]
fn create_symlink(target: &str, link_path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link_path)
}

#[cfg(test)]
mod tests {
    use super::ln;
    use std::fs;

    #[test]
    fn symbolic_link_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, "hi").unwrap();

        let result = ln(
            &[
                "-s".into(),
                target.to_string_lossy().into_owned(),
                link.to_string_lossy().into_owned(),
            ],
            &mut &b""[..],
            &mut std::io::sink(),
        );

        #[cfg(unix)]
        {
            assert_eq!(
                result.exit_code,
                0,
                "stderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        }

        #[cfg(windows)]
        {
            assert!(
                result.exit_code == 0
                    || String::from_utf8_lossy(&result.stderr).contains("1314")
                    || String::from_utf8_lossy(&result.stderr)
                        .contains("A required privilege is not held by the client"),
                "stderr: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[test]
    fn force_replaces_existing_link() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();

        let initial = ln(
            &[
                "-s".into(),
                first.to_string_lossy().into_owned(),
                link.to_string_lossy().into_owned(),
            ],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        #[cfg(unix)]
        assert_eq!(
            initial.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&initial.stderr)
        );

        #[cfg(windows)]
        if initial.exit_code != 0 {
            let stderr = String::from_utf8_lossy(&initial.stderr);
            assert!(
                stderr.contains("1314") || stderr.contains("A required privilege is not held"),
                "stderr: {stderr}"
            );
            return;
        }

        let replaced = ln(
            &[
                "-f".into(),
                "-s".into(),
                second.to_string_lossy().into_owned(),
                link.to_string_lossy().into_owned(),
            ],
            &mut &b""[..],
            &mut std::io::sink(),
        );

        assert_eq!(
            replaced.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&replaced.stderr)
        );
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&link).unwrap(), "second");
    }
}
