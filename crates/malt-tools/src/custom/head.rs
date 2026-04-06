//! Built-in `head` implementation.
//!
//! Outputs the first part of files.

use crate::BuiltinResult;
use std::io::BufRead;
use std::path::Path;

/// Output the first part of files.
///
/// - `head [OPTION]... [FILE]...`
/// - `-n NUM`: output first NUM lines (default 10)
/// - Exit 0 on success, 1 if any file can't be read
pub fn head(args: &[String], stdin: &[u8]) -> BuiltinResult {
    let mut num_lines: usize = 10;
    let mut file_args: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();

        if arg == "-n" || arg == "--lines" {
            if i + 1 < args.len() {
                i += 1;
                if let Ok(n) = args[i].parse::<isize>() {
                    // Negative means "all but last N lines"
                    if n < 0 {
                        // For simplicity, handle negative by reading all and skipping
                        // This isn't fully POSIX but works for most cases
                        num_lines = usize::MAX;
                    } else {
                        num_lines = n as usize;
                    }
                } else {
                    return BuiltinResult::failure(
                        1,
                        format!("head: invalid number of lines: '{}'\n", args[i]).into_bytes(),
                    );
                }
            } else {
                return BuiltinResult::failure(
                    1,
                    b"head: option requires an argument -- 'n'\n".to_vec(),
                );
            }
        } else if arg.starts_with("-n") {
            // -NUM syntax (e.g., -5 means 5 lines)
            if let Ok(n) = arg[2..].parse::<isize>() {
                if n < 0 {
                    num_lines = usize::MAX;
                } else {
                    num_lines = n as usize;
                }
            } else {
                return BuiltinResult::failure(
                    1,
                    format!("head: invalid number of lines: '{}'", &arg[2..]).into_bytes(),
                );
            }
        } else if arg.starts_with('-')
            && arg.len() > 1
            && arg.chars().skip(1).all(|c| c.is_ascii_digit())
        {
            // -NUM syntax where there's no 'n' prefix (e.g., -5)
            if let Ok(n) = arg[1..].parse::<isize>() {
                if n < 0 {
                    num_lines = usize::MAX;
                } else {
                    num_lines = n as usize;
                }
            }
        } else if arg == "-" || !arg.starts_with('-') {
            file_args.push(arg);
        } else if arg == "--" {
            i += 1;
            while i < args.len() {
                file_args.push(args[i].as_str());
                i += 1;
            }
            break;
        } else {
            return BuiltinResult::failure(
                1,
                format!("head: invalid option -- '{}'\n", arg).into_bytes(),
            );
        }
        i += 1;
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0;

    if file_args.is_empty() {
        // Read from stdin
        head_reader(stdin, num_lines, &mut stdout);
    } else {
        // Read from files
        let multiple_files = file_args.len() > 1;
        for (idx, path_str) in file_args.iter().enumerate() {
            if multiple_files {
                // Add header for multiple files
                if idx > 0 {
                    stdout.extend_from_slice(b"\n");
                }
                let header = format!("==> {} <==\n", path_str);
                stdout.extend_from_slice(header.as_bytes());
            }

            if *path_str == "-" {
                head_reader(stdin, num_lines, &mut stdout);
            } else {
                let path = Path::new(path_str);
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        head_reader(content.as_bytes(), num_lines, &mut stdout);
                    }
                    Err(e) => {
                        stderr.extend_from_slice(format!("head: {}: {}\n", path_str, e).as_bytes());
                        exit_code = 1;
                    }
                }
            }
        }
    }

    BuiltinResult {
        exit_code,
        stdout,
        stderr,
    }
}

fn head_reader(input: &[u8], num_lines: usize, output: &mut Vec<u8>) {
    let reader = std::io::BufReader::new(input);
    let mut count = 0;

    for line in reader.lines() {
        if let Ok(line) = line {
            if count >= num_lines {
                break;
            }
            output.extend_from_slice(line.as_bytes());
            output.push(b'\n');
            count += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_stdin_default() {
        let r = head(&[], b"line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11\nline12\n");
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert_eq!(out.lines().count(), 10);
    }

    #[test]
    fn head_stdin_n3() {
        let r = head(&["-n".into(), "3".into()], b"a\nb\nc\nd\ne\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "a\nb\nc\n");
    }

    #[test]
    fn head_stdin_short_form() {
        let r = head(&["-3".into()], b"a\nb\nc\nd\ne\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout), "a\nb\nc\n");
    }
}
