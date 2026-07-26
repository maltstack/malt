//! Built-in `sleep` implementation.
//!
//! Supports one or more numeric duration operands, interpreted as seconds and
//! summed before sleeping.

use crate::BuiltinResult;
use std::thread;
use std::time::Duration;

/// Sleep for the requested duration.
///
/// Exit 0 on success, 1 on invalid input.
pub fn sleep(
    args: &[String],
    _stdin: &mut dyn std::io::Read,
    _stdout: &mut dyn std::io::Write,
) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::failure(1, b"sleep: missing operand\n".to_vec());
    }

    let mut total_seconds = 0.0f64;
    for arg in args {
        let seconds = match parse_seconds(arg) {
            Ok(seconds) => seconds,
            Err(message) => {
                return BuiltinResult::failure(1, format!("sleep: {message}\n").into_bytes());
            }
        };
        total_seconds += seconds;
    }

    thread::sleep(Duration::from_secs_f64(total_seconds));
    BuiltinResult::success(Vec::new())
}

fn parse_seconds(arg: &str) -> Result<f64, String> {
    let seconds = arg
        .parse::<f64>()
        .map_err(|_| format!("invalid time interval '{arg}'"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!("invalid time interval '{arg}'"));
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn sleep_requires_operand() {
        let result = sleep(&[], &mut &b""[..], &mut std::io::sink());
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stdout, Vec::<u8>::new());
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            "sleep: missing operand\n"
        );
    }

    #[test]
    fn sleep_rejects_invalid_interval() {
        let result = sleep(&["abc".into()], &mut &b""[..], &mut std::io::sink());
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            String::from_utf8_lossy(&result.stderr),
            "sleep: invalid time interval 'abc'\n"
        );
    }

    #[test]
    fn sleep_sums_multiple_operands() {
        let start = Instant::now();
        let result = sleep(
            &["0.02".into(), "0.03".into()],
            &mut &b""[..],
            &mut std::io::sink(),
        );
        let elapsed = start.elapsed();

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert!(
            elapsed >= Duration::from_millis(40),
            "sleep returned too quickly: {elapsed:?}"
        );
    }
}
