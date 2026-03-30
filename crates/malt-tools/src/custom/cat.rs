//! Built-in `cat` implementation.

use crate::BuiltinResult;

/// Concatenate files and print to stdout.
///
/// - No args or `-`: read from stdin
/// - File args: read each file, concatenate to stdout
/// - `-n`: number all output lines
/// - Error on missing files (stderr message, exit 1)
pub fn cat(args: &[String], stdin: &[u8]) -> BuiltinResult {
    let mut number_lines = false;
    let mut file_args: Vec<&str> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-n" => number_lines = true,
            other => file_args.push(other),
        }
    }

    if file_args.is_empty() {
        // POSIX: no file operands -> read from standard input.
        let data = stdin.to_vec();
        if number_lines {
            return BuiltinResult::success(number_output(&data));
        }
        return BuiltinResult::success(data);
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0i32;

    for path in &file_args {
        if *path == "-" {
            // POSIX: `-` means read stdin.
            stdout.extend_from_slice(stdin);
            continue;
        }
        if *path == "/dev/null" {
            continue;
        }
        match std::fs::read(path) {
            Ok(data) => stdout.extend_from_slice(&data),
            Err(e) => {
                let msg = format!("cat: {}: {}\n", path, e);
                stderr.extend_from_slice(msg.as_bytes());
                exit_code = 1;
            }
        }
    }

    if number_lines {
        stdout = number_output(&stdout);
    }

    BuiltinResult {
        exit_code,
        stdout,
        stderr,
    }
}

/// Number each line in the output (1-indexed, right-aligned 6-char field).
fn number_output(data: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(data);
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let numbered = format!("{:>6}\t{}\n", i + 1, line);
        out.extend_from_slice(numbered.as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_stdin_passthrough() {
        let r = cat(&[], b"hello\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, b"hello\n");
    }

    #[test]
    fn cat_no_args_no_stdin() {
        let r = cat(&[], &[]);
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn cat_dash_reads_stdin() {
        let r = cat(&["-".into()], b"from stdin\n");
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, b"from stdin\n");
    }

    #[test]
    fn cat_number_lines() {
        let r = cat(&["-n".into()], b"aaa\nbbb\n");
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("1\taaa"));
        assert!(out.contains("2\tbbb"));
    }

    #[test]
    fn cat_missing_file() {
        let r = cat(&["/nonexistent/file.txt".into()], &[]);
        assert_eq!(r.exit_code, 1);
        assert!(!r.stderr.is_empty());
    }

    #[test]
    fn cat_dev_null() {
        let r = cat(&["/dev/null".into()], &[]);
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
    }
}
