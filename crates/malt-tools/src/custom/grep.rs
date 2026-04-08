//! Built-in `grep` implementation.
//!
//! Supports `-i`, `-v`, `-c`, `-n`, `-l` flags and regex patterns via the
//! `regex` crate.

use crate::BuiltinResult;
use regex::RegexBuilder;
use std::io::BufRead;
use std::path::Path;

/// Search for pattern matches in files or stdin.
///
/// - `grep PATTERN [FILE...]`
/// - No file: read from stdin
/// - `-i`: case insensitive
/// - `-v`: invert match
/// - `-c`: count matches only
/// - `-n`: line numbers
/// - `-l`: list matching filenames only
/// - Exit 0 if match found, 1 if not, 2 on error
pub fn grep(args: &[String], stdin: &[u8]) -> BuiltinResult {
    let mut case_insensitive = false;
    let mut invert = false;
    let mut count_only = false;
    let mut line_numbers = false;
    let mut files_only = false;
    let mut pattern_str: Option<String> = None;
    let mut file_args: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            i += 1;
            while i < args.len() {
                if pattern_str.is_none() {
                    pattern_str = Some(args[i].clone());
                } else {
                    file_args.push(args[i].as_str());
                }
                i += 1;
            }
            break;
        }

        if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
            let mut chars = arg[1..].chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    'i' => case_insensitive = true,
                    'v' => invert = true,
                    'c' => count_only = true,
                    'n' => line_numbers = true,
                    'l' => files_only = true,
                    'e' => {
                        let rest: String = chars.collect();
                        if !rest.is_empty() {
                            pattern_str = Some(rest);
                        } else {
                            i += 1;
                            if i >= args.len() {
                                return BuiltinResult::failure(
                                    2,
                                    b"grep: option requires an argument -- 'e'\n".to_vec(),
                                );
                            }
                            pattern_str = Some(args[i].clone());
                        }
                        break;
                    }
                    _ => {
                        let msg = format!("grep: unknown option: -{}\n", ch);
                        return BuiltinResult::failure(2, msg.into_bytes());
                    }
                }
            }
        } else if pattern_str.is_none() {
            pattern_str = Some(args[i].clone());
        } else {
            file_args.push(args[i].as_str());
        }
        i += 1;
    }

    let pattern_str = match pattern_str {
        Some(p) => p,
        None => {
            return BuiltinResult::failure(
                2,
                b"grep: usage: grep [OPTIONS] PATTERN [FILE...]\n".to_vec(),
            );
        }
    };

    let re = match RegexBuilder::new(&pattern_str)
        .case_insensitive(case_insensitive)
        .size_limit(1_000_000)
        .build()
    {
        Ok(re) => re,
        Err(e) => {
            let msg = format!("grep: invalid regex '{}': {}\n", pattern_str, e);
            return BuiltinResult::failure(2, msg.into_bytes());
        }
    };

    if file_args.is_empty() {
        return grep_reader(
            stdin,
            &re,
            invert,
            count_only,
            line_numbers,
            files_only,
            None,
        );
    }

    let show_filename = file_args.len() > 1;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut any_match = false;
    let mut had_error = false;

    for path_str in &file_args {
        let path = Path::new(path_str);
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("grep: {}: {}\n", path_str, e);
                stderr.extend_from_slice(msg.as_bytes());
                had_error = true;
                continue;
            }
        };

        let label = if show_filename { Some(*path_str) } else { None };
        let result = grep_reader(
            &data,
            &re,
            invert,
            count_only,
            line_numbers,
            files_only,
            label,
        );

        if result.exit_code == 0 {
            any_match = true;
            if files_only {
                let line = format!("{}\n", path_str);
                stdout.extend_from_slice(line.as_bytes());
            } else {
                stdout.extend_from_slice(&result.stdout);
            }
        } else if !count_only {
            stdout.extend_from_slice(&result.stdout);
        } else {
            // count mode, no matches
            stdout.extend_from_slice(&result.stdout);
        }
    }

    let exit_code = if any_match {
        0
    } else if had_error {
        2
    } else {
        1
    };

    BuiltinResult {
        exit_code,
        stdout,
        stderr,
    }
}

fn grep_reader(
    data: &[u8],
    re: &regex::Regex,
    invert: bool,
    count_only: bool,
    line_numbers: bool,
    files_only: bool,
    filename: Option<&str>,
) -> BuiltinResult {
    let cursor = std::io::Cursor::new(data);
    let mut stdout = Vec::new();
    let mut match_count = 0usize;

    for (idx, line_result) in cursor.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };

        let matched = re.is_match(&line);
        let matched = if invert { !matched } else { matched };

        if matched {
            match_count += 1;

            if files_only {
                // Just need to know there is a match; caller handles filename output.
                return BuiltinResult::success(Vec::new());
            }

            if !count_only {
                if let Some(fname) = filename {
                    stdout.extend_from_slice(fname.as_bytes());
                    stdout.push(b':');
                }
                if line_numbers {
                    let num = format!("{}:", idx + 1);
                    stdout.extend_from_slice(num.as_bytes());
                }
                stdout.extend_from_slice(line.as_bytes());
                stdout.push(b'\n');
            }
        }
    }

    if count_only {
        if let Some(fname) = filename {
            stdout.extend_from_slice(fname.as_bytes());
            stdout.push(b':');
        }
        let count_str = format!("{}\n", match_count);
        stdout.extend_from_slice(count_str.as_bytes());
    }

    let exit_code = if match_count > 0 { 0 } else { 1 };
    BuiltinResult {
        exit_code,
        stdout,
        stderr: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_basic_match() {
        let r = grep(&["hello".into()], b"hello world\ngoodbye\nhello again\n");
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("hello world"));
        assert!(out.contains("hello again"));
        assert!(!out.contains("goodbye"));
    }

    #[test]
    fn grep_no_match() {
        let r = grep(&["xyz".into()], b"hello\nworld\n");
        assert_eq!(r.exit_code, 1);
    }

    #[test]
    fn grep_case_insensitive() {
        let r = grep(
            &["-i".into(), "hello".into()],
            b"Hello World\nhello world\nHELLO\n",
        );
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn grep_invert() {
        let r = grep(&["-v".into(), "hello".into()], b"hello\nworld\n");
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(!out.contains("hello"));
        assert!(out.contains("world"));
    }

    #[test]
    fn grep_count() {
        let r = grep(&["-c".into(), "hello".into()], b"hello\nhello\nworld\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "2");
    }

    #[test]
    fn grep_line_numbers() {
        let r = grep(&["-n".into(), "aaa".into()], b"aaa\nbbb\naaa\n");
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("1:aaa"));
        assert!(out.contains("3:aaa"));
    }

    #[test]
    fn grep_no_pattern() {
        let r = grep(&[], &[]);
        assert_eq!(r.exit_code, 2);
    }

    #[test]
    fn grep_invalid_regex() {
        let r = grep(&["[invalid".into()], b"test\n");
        assert_eq!(r.exit_code, 2);
    }

    #[test]
    fn grep_e_option_uses_following_pattern() {
        let r = grep(&["-e".into(), "^.$".into()], b".\n..\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), ".\n");
    }

    #[test]
    fn grep_combined_flags_can_be_followed_by_e_pattern() {
        let r = grep(&["-in".into(), "-e".into(), "hello".into()], b"HELLO\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "1:HELLO\n");
    }
}
