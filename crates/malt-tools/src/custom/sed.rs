//! Built-in `sed` implementation (minimal).
//!
//! Supports basic substitution patterns used by Smoosh tests.

use crate::{emit, BuiltinResult};
use regex::Regex;
use std::io::BufRead;
use std::path::Path;

/// Stream editor for filtering and transforming text.
///
/// Currently supports:
/// - `s/pattern/replacement/` - substitute first match on each line
/// - `s/pattern/replacement/g` - substitute all matches on each line
/// - `s/pattern/replacement/N` - substitute Nth match on each line
/// - Capture groups `\(...\)` and backreferences `\1`, `\2`, etc.
///
/// Exit 0 on success, 1 on error
pub fn sed(
    args: &[String],
    stdin: &mut dyn std::io::Read,
    stdout_writer: &mut dyn std::io::Write,
) -> BuiltinResult {
    if args.is_empty() {
        // No script: just copy stdin to stdout (cat-like behavior), streaming
        // each chunk as it is read instead of only after stdin ends.
        return BuiltinResult::success(stream_copy(stdin, stdout_writer));
    }

    // Check for -e or direct script argument
    let mut script: Option<&str> = None;
    let mut file_args: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();

        if arg == "-e" || arg == "--expression" {
            if i + 1 < args.len() {
                script = Some(args[i + 1].as_str());
                i += 2;
                continue;
            } else {
                return BuiltinResult::failure(
                    1,
                    b"sed: option requires an argument -- 'e'\n".to_vec(),
                );
            }
        } else if arg == "-n" || arg == "--quiet" || arg == "--silent" {
            // Silent mode - not fully implemented
            i += 1;
        } else if arg == "-i" || arg.starts_with("-i") {
            // In-place editing - not supported for in-process tool
            i += 1;
        } else if arg == "--" {
            i += 1;
            while i < args.len() {
                if script.is_none() && !args[i].starts_with('-') {
                    script = Some(args[i].as_str());
                } else {
                    file_args.push(args[i].as_str());
                }
                i += 1;
            }
            break;
        } else if !arg.starts_with('-') {
            if script.is_none() {
                script = Some(arg);
            } else {
                file_args.push(arg);
            }
        } else {
            return BuiltinResult::failure(
                1,
                format!("sed: invalid option -- '{}'\n", arg).into_bytes(),
            );
        }
        i += 1;
    }

    let script = script.unwrap_or("");

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    if file_args.is_empty() {
        // Stream: each transformed line is written to `stdout_writer` as
        // soon as it is produced, instead of only after stdin reaches
        // end-of-input.
        let reader = std::io::BufReader::new(&mut *stdin);
        // map_while rather than flatten: on a reader that keeps returning
        // Err, flatten spins forever instead of stopping.
        for line in reader.lines().map_while(Result::ok) {
            match apply_script(&line, script) {
                Ok(result) => {
                    let mut rendered = result.into_bytes();
                    rendered.push(b'\n');
                    let _ = stdout_writer.write_all(&rendered);
                    stdout.extend_from_slice(&rendered);
                }
                Err(e) => {
                    stderr.extend_from_slice(format!("sed: {}\n", e).as_bytes());
                    exit_code = 1;
                }
            }
        }
        return BuiltinResult {
            exit_code,
            stdout,
            stderr,
        };
    }

    // Files are read whole up front, so there is nothing to stream
    // incrementally here -- process and write once, below, like `cat`'s and
    // `grep`'s file-operand paths.
    let mut input_lines: Vec<String> = Vec::new();
    for path_str in file_args {
        let path = Path::new(path_str);
        match std::fs::read_to_string(path) {
            Ok(content) => {
                for line in content.lines() {
                    input_lines.push(line.to_string());
                }
            }
            Err(e) => {
                stderr.extend_from_slice(format!("sed: {}: {}\n", path_str, e).as_bytes());
                exit_code = 1;
            }
        }
    }

    for line in input_lines {
        match apply_script(&line, script) {
            Ok(result) => {
                stdout.extend_from_slice(result.as_bytes());
                stdout.push(b'\n');
            }
            Err(e) => {
                stderr.extend_from_slice(format!("sed: {}\n", e).as_bytes());
                exit_code = 1;
            }
        }
    }

    emit(
        stdout_writer,
        BuiltinResult {
            exit_code,
            stdout,
            stderr,
        },
    )
}

/// Read `reader` to end-of-input, writing each chunk to `writer` as soon as
/// it is read and accumulating the same bytes for the returned buffer (used
/// for redirect/pipeline capture, which need the complete output regardless
/// of whether anything was streaming).
fn stream_copy(reader: &mut dyn std::io::Read, writer: &mut dyn std::io::Write) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return collected,
            Ok(n) => {
                let _ = writer.write_all(&buf[..n]);
                collected.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return collected,
        }
    }
}

/// Apply a sed script to a single line.
fn apply_script(line: &str, script: &str) -> Result<String, String> {
    // Handle substitution: s/pattern/replacement/flags
    if script.starts_with("s") {
        let delim = script.chars().nth(1).unwrap_or('/');
        let parts: Vec<&str> = script[2..].split(delim).collect();

        if parts.len() < 2 {
            return Err("invalid substitution pattern".to_string());
        }

        let pattern = parts[0];
        let replacement = parts[1];
        let flags = if parts.len() > 2 { parts[2] } else { "" };

        // Parse flags
        let global = flags.contains('g');
        let nth: Option<usize> = flags
            .chars()
            .find(|c| c.is_ascii_digit())
            .and_then(|c| c.to_digit(10))
            .map(|n| n as usize);

        // Convert sed pattern to regex:
        // 1. Convert \(...\) to (... ) for capture groups
        // 2. Handle other sed-specific syntax
        let regex_pattern = convert_sed_pattern_to_regex(pattern);

        if global {
            // Replace all occurrences
            let regex = Regex::new(&regex_pattern).map_err(|e| format!("invalid regex: {}", e))?;

            // Replace with backreference handling
            let result = regex
                .replace_all(line, |caps: &regex::Captures| {
                    expand_backreferences(replacement, caps)
                })
                .to_string();
            Ok(result)
        } else if let Some(n) = nth {
            // Replace Nth occurrence only
            let regex = Regex::new(&regex_pattern).map_err(|e| format!("invalid regex: {}", e))?;
            let mut count = 0;
            let mut result = line.to_string();

            for mat in regex.find_iter(line) {
                count += 1;
                if count == n {
                    // For nth replacement, we need to extract captures from this specific match
                    // We'll use a temp approach: create the replacement by manually scanning
                    let replacement_expanded =
                        if let Some(caps) = regex.captures(&line[mat.start()..mat.end()]) {
                            expand_backreferences(replacement, &caps)
                        } else {
                            // No captures - use replacement as-is but still expand \n etc
                            replacement.to_string()
                        };
                    result = format!(
                        "{}{}{}",
                        &line[..mat.start()],
                        replacement_expanded,
                        &line[mat.end()..]
                    );
                    break;
                }
            }
            Ok(result)
        } else {
            // Replace first occurrence only
            let regex = Regex::new(&regex_pattern).map_err(|e| format!("invalid regex: {}", e))?;

            if let Some(caps) = regex.captures(line) {
                let Some(mat) = caps.get(0) else {
                    return Ok(line.to_string());
                };
                let replacement_expanded = expand_backreferences(replacement, &caps);
                let result = format!(
                    "{}{}{}",
                    &line[..mat.start()],
                    replacement_expanded,
                    &line[mat.end()..]
                );
                Ok(result)
            } else {
                Ok(line.to_string())
            }
        }
    } else if script == "q" || script.starts_with("q ") {
        // Quit command - would need to be handled at stream level
        Ok(line.to_string())
    } else if script == "d" {
        // Delete line
        Ok(String::new())
    } else if script == "p" {
        // Print line
        Ok(line.to_string())
    } else {
        // Unknown script - just pass through
        Ok(line.to_string())
    }
}

/// Convert sed basic regular expression pattern to Rust regex.
/// Handles:
/// - \(...\) capture groups -> (...)
/// - . matches any character
/// - * matches zero or more
/// - character classes [abc] or [0-9]
/// - other sed BRE syntax
fn convert_sed_pattern_to_regex(pattern: &str) -> String {
    let mut result = String::new();
    let mut chars = pattern.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('(') => {
                    // \( starts a capture group
                    result.push('(');
                }
                Some(')') => {
                    // \) ends a capture group
                    result.push(')');
                }
                Some(d) if d.is_ascii_digit() && d != '0' => {
                    // Backreference in pattern - sed doesn't support this in patterns,
                    // but we handle it anyway (will be treated as literal)
                    result.push('\\');
                    result.push(d);
                }
                Some(other) => {
                    // Escaped character - pass through with escape
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    result.push('\\');
                }
            }
        } else if c == '[' {
            // Start of character class - copy until ]
            result.push('[');
            for ch in chars.by_ref() {
                result.push(ch);
                if ch == ']' {
                    break;
                }
            }
        } else if c == '.' {
            // In sed BRE, . matches any character (same in regex)
            result.push('.');
        } else if c == '*' {
            // In sed BRE, * matches zero or more (same in regex)
            result.push('*');
        } else if c == '^' || c == '$' {
            // Anchors
            result.push(c);
        } else if "+?{}|".contains(c) {
            // These need escaping in regex to be literal
            result.push('\\');
            result.push(c);
        } else {
            // Regular character
            result.push(c);
        }
    }

    result
}

/// Expand backreferences like \1, \2 in replacement string using captures.
fn expand_backreferences(replacement: &str, caps: &regex::Captures) -> String {
    let mut result = String::new();
    let mut chars = replacement.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(d) if d.is_ascii_digit() => {
                    // \N is a backreference to capture group N
                    if let Some(group_num) = d.to_digit(10) {
                        if let Some(group) = caps.get(group_num as usize) {
                            result.push_str(group.as_str());
                        }
                    }
                    // If group doesn't exist, replace with empty string
                }
                Some(other) => {
                    // Other escaped character
                    result.push('\\');
                    result.push(other);
                }
                None => {
                    result.push('\\');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sed_streams_each_transformed_line_as_it_is_read_not_only_at_the_end() {
        let mut written = Vec::new();
        let r = sed(
            &["s/foo/bar/".into()],
            &mut &b"foo one\nfoo two\n"[..],
            &mut written,
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, b"bar one\nbar two\n");
        assert_eq!(
            written, b"bar one\nbar two\n",
            "each transformed line must have been streamed to the writer"
        );
    }

    #[test]
    fn sed_substitute_first() {
        let r = sed(
            &["s/foo/bar/".into()],
            &mut &b"foo foo foo\n"[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "bar foo foo\n");
    }

    #[test]
    fn sed_substitute_global() {
        let r = sed(
            &["s/foo/bar/g".into()],
            &mut &b"foo foo foo\n"[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "bar bar bar\n");
    }

    #[test]
    fn sed_substitute_nth() {
        let r = sed(
            &["s/foo/bar/2".into()],
            &mut &b"foo foo foo\n"[..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "foo bar foo\n");
    }

    #[test]
    fn sed_capture_groups() {
        // Test capture groups and backreferences
        let input = "0m1.234s 0m2.567s\n";
        let r = sed(
            &["s/\\([0-9]*\\)m\\([0-9]*\\).\\([0-9]*\\)s.*/\\1/".into()],
            &mut input.as_bytes(),
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "0\n");
    }

    #[test]
    fn sed_backreference_minutes() {
        // Extract minutes from timing string
        let input = "0m1.234s\n";
        let r = sed(
            &["s/\\([0-9]*\\)m\\([0-9]*\\).\\([0-9]*\\)s.*/\\1/".into()],
            &mut input.as_bytes(),
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "0\n");
    }

    #[test]
    fn sed_backreference_seconds() {
        // Extract seconds from timing string
        let input = "0m1.234s\n";
        let r = sed(
            &["s/\\([0-9]*\\)m\\([0-9]*\\).\\([0-9]*\\)s.*/\\2/".into()],
            &mut input.as_bytes(),
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "1\n");
    }

    #[test]
    fn sed_backreference_fractional() {
        // Extract fractional from timing string
        let input = "0m1.234s\n";
        let r = sed(
            &["s/\\([0-9]*\\)m\\([0-9]*\\).\\([0-9]*\\)s.*/\\3/".into()],
            &mut input.as_bytes(),
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "234\n");
    }
}
