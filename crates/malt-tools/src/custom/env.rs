//! Built-in `env` implementation.

use crate::BuiltinResult;

/// Print all environment variables.
///
/// - No args: print all environment variables (`KEY=VALUE` per line, sorted)
/// - With args: not yet supported (returns error)
pub fn env_cmd(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    if args.is_empty() {
        // Print all environment variables, one per line, sorted.
        let mut lines: Vec<String> = std::env::vars()
            .map(|(k, v)| format!("{}={}\n", k, v))
            .collect();
        lines.sort_unstable();
        let out: String = lines.concat();
        return BuiltinResult::success(out.into_bytes());
    }

    // `env VAR=value cmd` form -- not yet implemented.
    BuiltinResult::failure(1, b"env: command execution not yet supported\n".to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_prints_vars() {
        let r = env_cmd(&[], &[]);
        assert_eq!(r.exit_code, 0);
        let out = String::from_utf8_lossy(&r.stdout);
        // Should contain at least PATH (or Path on Windows)
        assert!(
            out.contains("PATH") || out.contains("Path"),
            "expected PATH in output"
        );
    }

    #[test]
    fn env_output_is_sorted() {
        let r = env_cmd(&[], &[]);
        let out = String::from_utf8_lossy(&r.stdout);
        let lines: Vec<&str> = out.lines().collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn env_with_args_unsupported() {
        let r = env_cmd(&["FOO=bar".into()], &[]);
        assert_ne!(r.exit_code, 0);
    }
}
