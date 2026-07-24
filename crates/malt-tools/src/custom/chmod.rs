//! Built-in `chmod` implementation (minimal).
//!
//! Supports:
//! - Octal mode (e.g., 755, 644)
//! - Symbolic mode (e.g., +x, -w) - basic support only
//!
//! Note: On Windows, this has limited effect - primarily for compatibility.

use crate::BuiltinResult;
use malt_platform::fs;
use std::path::Path;

/// Change file mode bits.
///
/// Exit 0 on success, 1 on error.
pub fn chmod(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    let mut _recursive = false;
    let mut mode_str: Option<&str> = None;
    let mut paths: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        if mode_str.is_none() && looks_like_shorthand_mode(&args[i]) {
            mode_str = Some(&args[i]);
            i += 1;
            continue;
        }

        match args[i].as_str() {
            "-R" | "-r" | "--recursive" => _recursive = true,
            "-f" | "--force" => { /* ignore errors */ }
            "-v" | "--verbose" => { /* ignored */ }
            _ => {
                if mode_str.is_none() {
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

    let mode_str = mode_str.unwrap();
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

        let current_mode = match fs::get_mode(p) {
            Ok(mode) => mode,
            Err(e) => {
                stderr.extend_from_slice(
                    format!("chmod: cannot access '{}': {}\n", path, e).as_bytes(),
                );
                exit_code = 1;
                continue;
            }
        };

        let mode_val = match parse_mode(mode_str, current_mode) {
            Ok(mode) => mode,
            Err(message) => {
                stderr.extend_from_slice(format!("chmod: {message}\n").as_bytes());
                exit_code = 1;
                continue;
            }
        };

        if let Err(e) = fs::set_mode(p, mode_val) {
            stderr.extend_from_slice(
                format!("chmod: changing permissions of '{}': {}\n", path, e).as_bytes(),
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

fn looks_like_shorthand_mode(arg: &str) -> bool {
    let mut chars = arg.chars();
    matches!(chars.next(), Some('+' | '-' | '='))
        && chars.clone().next().is_some()
        && chars.all(|ch| matches!(ch, 'r' | 'w' | 'x'))
}

fn parse_mode(mode: &str, current: u32) -> Result<u32, String> {
    if let Ok(octal) = u32::from_str_radix(mode, 8) {
        return Ok(octal & 0o777);
    }

    apply_symbolic_mode(mode, current)
}

fn apply_symbolic_mode(mode: &str, current: u32) -> Result<u32, String> {
    let mut chars = mode.chars().peekable();
    let mut who_bits = 0u32;

    while let Some(ch) = chars.peek().copied() {
        match ch {
            'u' => {
                who_bits |= 0o700;
                chars.next();
            }
            'g' => {
                who_bits |= 0o070;
                chars.next();
            }
            'o' => {
                who_bits |= 0o007;
                chars.next();
            }
            'a' => {
                who_bits |= 0o777;
                chars.next();
            }
            '+' | '-' | '=' => break,
            _ => return Err(format!("invalid mode: {mode}")),
        }
    }

    if who_bits == 0 {
        who_bits = 0o777;
    }

    let op = chars
        .next()
        .ok_or_else(|| format!("invalid mode: {mode}"))?;
    if !matches!(op, '+' | '-' | '=') {
        return Err(format!("invalid mode: {mode}"));
    }

    let mut perm_mask = 0u32;
    for ch in chars {
        perm_mask |= match ch {
            'r' => mask_for(who_bits, 0o444),
            'w' => mask_for(who_bits, 0o222),
            'x' => mask_for(who_bits, 0o111),
            _ => return Err(format!("invalid mode: {mode}")),
        };
    }

    let next = match op {
        '+' => current | perm_mask,
        '-' => current & !perm_mask,
        '=' => (current & !mask_for(who_bits, 0o777)) | perm_mask,
        _ => unreachable!(),
    };

    Ok(next & 0o777)
}

fn mask_for(who_bits: u32, perm_bits: u32) -> u32 {
    who_bits & perm_bits
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

    #[test]
    fn chmod_minus_r_is_treated_as_mode_not_recursive_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.sh");
        fs::write(&path, "echo hi\n").unwrap();

        let result = chmod(&["-r".into(), path.to_string_lossy().into_owned()], b"");

        assert_eq!(
            result.exit_code,
            0,
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            !malt_platform::fs::is_readable(&path),
            "mode should remove read permission"
        );
    }
}
