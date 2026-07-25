//! Built-in `wc` implementation.

use crate::{emit, BuiltinResult};
use std::path::Path;

/// Count lines, words, and bytes.
///
/// - `wc [FILE...]`
/// - No file: read from stdin
/// - Default: print lines, words, bytes
/// - `-l`: lines only
/// - `-w`: words only
/// - `-c`: bytes only
/// - `-m`: characters only
///
/// `wc` produces exactly one summary line per operand, computed only once
/// all of its input has been read -- there is no meaningful way to emit a
/// count before that point, unlike `cat`/`grep`/`sed`/`head`. It still takes
/// the writer parameter, and still uses it (via [`emit`]) for the one thing
/// that is true of every tool: its result reaches the caller through the
/// same seam, whether or not anything was streamed incrementally.
pub fn wc(
    args: &[String],
    stdin: &mut dyn std::io::Read,
    stdout_writer: &mut dyn std::io::Write,
) -> BuiltinResult {
    let mut lines_only = false;
    let mut words_only = false;
    let mut bytes_only = false;
    let mut chars_only = false;
    let mut file_args: Vec<&str> = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'l' => lines_only = true,
                    'w' => words_only = true,
                    'c' => bytes_only = true,
                    'm' => chars_only = true,
                    _ => {
                        let msg = format!("wc: unknown option: -{}\n", ch);
                        return emit(stdout_writer, BuiltinResult::failure(1, msg.into_bytes()));
                    }
                }
            }
        } else {
            file_args.push(arg.as_str());
        }
    }

    // If no specific flag, show all three (lines, words, bytes).
    let show_all = !lines_only && !words_only && !bytes_only && !chars_only;

    if file_args.is_empty() {
        let counts = count_data(&crate::read_all(stdin));
        let line = format_counts(
            &counts, show_all, lines_only, words_only, bytes_only, chars_only, None,
        );
        return emit(stdout_writer, BuiltinResult::success(line.into_bytes()));
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0i32;
    let mut totals = Counts::default();
    let multiple = file_args.len() > 1;

    for path_str in &file_args {
        let path = Path::new(path_str);
        match std::fs::read(path) {
            Ok(data) => {
                let counts = count_data(&data);
                totals.lines += counts.lines;
                totals.words += counts.words;
                totals.bytes += counts.bytes;
                totals.chars += counts.chars;
                let line = format_counts(
                    &counts,
                    show_all,
                    lines_only,
                    words_only,
                    bytes_only,
                    chars_only,
                    Some(path_str),
                );
                stdout.extend_from_slice(line.as_bytes());
            }
            Err(e) => {
                let msg = format!("wc: {}: {}\n", path_str, e);
                stderr.extend_from_slice(msg.as_bytes());
                exit_code = 1;
            }
        }
    }

    if multiple {
        let line = format_counts(
            &totals,
            show_all,
            lines_only,
            words_only,
            bytes_only,
            chars_only,
            Some("total"),
        );
        stdout.extend_from_slice(line.as_bytes());
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

#[derive(Default)]
struct Counts {
    lines: usize,
    words: usize,
    bytes: usize,
    chars: usize,
}

fn count_data(data: &[u8]) -> Counts {
    let text = String::from_utf8_lossy(data);
    let lines = text.as_bytes().iter().filter(|&&b| b == b'\n').count();
    let words = text.split_whitespace().count();
    let bytes = data.len();
    let chars = text.chars().count();
    Counts {
        lines,
        words,
        bytes,
        chars,
    }
}

fn format_counts(
    counts: &Counts,
    show_all: bool,
    lines_only: bool,
    words_only: bool,
    bytes_only: bool,
    chars_only: bool,
    filename: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if show_all || lines_only {
        parts.push(format!("{}", counts.lines));
    }
    if show_all || words_only {
        parts.push(format!("{}", counts.words));
    }
    if show_all || bytes_only {
        parts.push(format!("{}", counts.bytes));
    }
    if chars_only {
        parts.push(format!("{}", counts.chars));
    }

    let mut line = parts.join(" ");
    if let Some(name) = filename {
        line.push(' ');
        line.push_str(name);
    }
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wc_stdin_default() {
        let r = wc(&[], &mut &b"hello world\ngoodbye\n"[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        // 2 lines, 3 words, some bytes
        assert!(out.contains("2"), "expected 2 lines in: {}", out);
        assert!(out.contains("3"), "expected 3 words in: {}", out);
    }

    #[test]
    fn wc_lines_only() {
        let r = wc(&["-l".into()], &mut &b"a\nb\nc\n"[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "3");
    }

    #[test]
    fn wc_words_only() {
        let r = wc(&["-w".into()], &mut &b"one two three\nfour\n"[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "4");
    }

    #[test]
    fn wc_bytes_only() {
        let data = b"hello\n";
        let r = wc(&["-c".into()], &mut &data[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert_eq!(
            String::from_utf8_lossy(&r.stdout).trim(),
            data.len().to_string()
        );
    }

    #[test]
    fn wc_chars_only() {
        let r = wc(&["-m".into()], &mut "hello\n".as_bytes(), &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&r.stdout).trim(), "6");
    }

    #[test]
    fn wc_empty_input() {
        let r = wc(&[], &mut &[][..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("0"));
    }
}
