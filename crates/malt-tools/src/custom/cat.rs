//! Built-in `cat` implementation.

use crate::{emit, BuiltinResult};
use std::io::{Read, Write};

/// Concatenate files and print to stdout.
///
/// - No args or `-`: read from stdin
/// - File args: read each file, concatenate to stdout
/// - `-n`: number all output lines
/// - Error on missing files (stderr message, exit 1)
pub fn cat(
    args: &[String],
    stdin: &mut dyn std::io::Read,
    stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
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
        if number_lines {
            // Numbering needs the whole buffer to compute line numbers, so
            // there is no incremental case to serve here.
            let data = crate::read_all(stdin);
            return emit(stdout, BuiltinResult::success(number_output(&data)));
        }
        // Stream straight through: each piece read from stdin is written to
        // stdout (and accumulated for the returned buffer) immediately,
        // instead of only after stdin reaches end-of-input. This is the
        // difference that makes `cat` fed input over time show each line
        // before the next one arrives (SC-008).
        let data = stream_copy(stdin, stdout);
        return BuiltinResult::success(data);
    }

    let mut collected = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_code = 0i32;

    for path in &file_args {
        if *path == "-" {
            // POSIX: `-` means read stdin.
            if number_lines {
                collected.extend_from_slice(&crate::read_all(stdin));
            } else {
                let data = stream_copy(stdin, stdout);
                collected.extend_from_slice(&data);
            }
            continue;
        }
        if *path == "/dev/null" {
            continue;
        }
        match std::fs::read(path) {
            Ok(data) => collected.extend_from_slice(&data),
            Err(e) => {
                let msg = format!("cat: {}: {}\n", path, e);
                stderr.extend_from_slice(msg.as_bytes());
                exit_code = 1;
            }
        }
    }

    if number_lines {
        collected = number_output(&collected);
        return emit(
            stdout,
            BuiltinResult {
                exit_code,
                stdout: collected,
                stderr,
            },
        );
    }

    // File contents were read whole (not streamed above) -- write them to
    // stdout now. Anything from a `-` operand was already streamed and must
    // not be written again.
    let _ = stdout.write_all(&collected);
    BuiltinResult {
        exit_code,
        stdout: collected,
        stderr,
    }
}

/// Read `reader` to end-of-input, writing each chunk to `writer` as soon as
/// it is read and accumulating the same bytes for the returned buffer (used
/// for redirect/pipeline capture, which need the complete output regardless
/// of whether anything was streaming).
fn stream_copy(reader: &mut dyn Read, writer: &mut dyn Write) -> Vec<u8> {
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
        let mut out = Vec::new();
        let r = cat(&[], &mut &b"hello\n"[..], &mut out);
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, b"hello\n");
        assert_eq!(out, b"hello\n", "streamed bytes must match the returned buffer");
    }

    #[test]
    fn cat_no_args_no_stdin() {
        let r = cat(&[], &mut &[][..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn cat_dash_reads_stdin() {
        let r = cat(&["-".into()], &mut &b"from stdin\n"[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, b"from stdin\n");
    }

    #[test]
    fn cat_number_lines() {
        let r = cat(&["-n".into()], &mut &b"aaa\nbbb\n"[..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("1\taaa"));
        assert!(out.contains("2\tbbb"));
    }

    #[test]
    fn cat_missing_file() {
        let r = cat(
            &["/nonexistent/file.txt".into()],
            &mut &[][..],
            &mut std::io::sink(),
        );
        assert_eq!(r.exit_code, 1);
        assert!(!r.stderr.is_empty());
    }

    #[test]
    fn cat_dev_null() {
        let r = cat(&["/dev/null".into()], &mut &[][..], &mut std::io::sink());
        assert_eq!(r.exit_code, 0);
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn cat_streams_each_chunk_as_it_is_read_not_only_at_the_end() {
        // A reader that yields one byte per call, so a single blocking
        // `read_to_end` would hide the incremental behavior this test
        // exists to catch.
        struct OneByteAtATime<'a>(&'a [u8]);
        impl<'a> std::io::Read for OneByteAtATime<'a> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }

        let mut written = Vec::new();
        let mut reader = OneByteAtATime(b"ab");
        let r = cat(&[], &mut reader, &mut written);
        assert_eq!(r.stdout, b"ab");
        assert_eq!(
            written, b"ab",
            "every byte the reader produced must have reached the writer"
        );
    }
}
