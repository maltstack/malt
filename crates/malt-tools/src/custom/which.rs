//! Built-in `which` implementation.

use crate::BuiltinResult;
use std::path::{Path, PathBuf};

#[cfg(unix)]
const PATH_SEP: char = ':';
#[cfg(windows)]
const PATH_SEP: char = ';';

/// Locate a command by searching PATH.
///
/// - For each arg: search PATH for executable
/// - Print path if found
/// - Exit 1 if any not found
/// - Windows: check PATHEXT extensions (.exe, .cmd, .bat, etc.)
pub fn which(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::failure(1, b"which: missing argument\n".to_vec());
    }

    let path_var = std::env::var("PATH").unwrap_or_default();
    let search_dirs: Vec<&Path> = path_var.split(PATH_SEP).map(Path::new).collect();

    let extensions = pathext_extensions();

    let mut stdout = Vec::new();
    let mut exit_code = 0i32;

    for name in args {
        match find_in_path(name, &search_dirs, &extensions) {
            Some(found) => {
                stdout.extend_from_slice(found.to_string_lossy().as_bytes());
                stdout.push(b'\n');
            }
            None => {
                exit_code = 1;
                let msg = format!("{}: not found\n", name);
                stdout.extend_from_slice(msg.as_bytes());
            }
        }
    }

    BuiltinResult {
        exit_code,
        stdout,
        stderr: Vec::new(),
    }
}

/// Collect executable extensions to try when resolving a command name.
///
/// On Windows, reads the `PATHEXT` environment variable (falling back to
/// `.COM;.EXE;.BAT;.CMD`) and returns a list of lowercase extensions
/// including the leading dot.
///
/// On Unix, returns an empty list -- extension probing is not needed.
fn pathext_extensions() -> Vec<String> {
    if cfg!(windows) {
        let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        raw.split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        Vec::new()
    }
}

fn find_in_path(name: &str, dirs: &[&Path], extensions: &[String]) -> Option<PathBuf> {
    // Absolute or relative path given directly
    if name.contains('/') || name.contains('\\') {
        let p = PathBuf::from(name);
        if p.is_file() && is_executable(&p) {
            return Some(p);
        }
        if let Some(found) = try_extensions(&p, extensions) {
            return Some(found);
        }
        return None;
    }

    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate);
        }
        if let Some(found) = try_extensions(&candidate, extensions) {
            return Some(found);
        }
    }
    None
}

/// Try appending each PATHEXT extension to `base` and return the first match.
fn try_extensions(base: &Path, extensions: &[String]) -> Option<PathBuf> {
    let base_str = base.to_string_lossy();
    for ext in extensions {
        let with_ext = PathBuf::from(format!("{}{}", base_str, ext));
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    malt_platform::io::is_executable(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_missing_arg() {
        let r = which(&[], &[]);
        assert_eq!(r.exit_code, 1);
    }

    #[test]
    fn which_not_found() {
        let r = which(&["definitely_not_a_real_tool_xyz_12345".into()], &[]);
        assert_eq!(r.exit_code, 1);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("not found"));
    }

    #[cfg(windows)]
    #[test]
    fn which_finds_cmd_on_windows() {
        let r = which(&["cmd".into()], &[]);
        let out = String::from_utf8_lossy(&r.stdout).to_ascii_lowercase();
        assert_eq!(r.exit_code, 0, "which cmd failed: {}", out);
        assert!(out.contains("cmd"), "expected cmd in: {}", out);
    }

    #[cfg(unix)]
    #[test]
    fn which_finds_ls_on_unix() {
        let r = which(&["ls".into()], &[]);
        let out = String::from_utf8_lossy(&r.stdout);
        assert_eq!(r.exit_code, 0, "which ls failed: {}", out);
        assert!(out.contains("/ls"), "expected path with /ls in: {}", out);
    }
}
