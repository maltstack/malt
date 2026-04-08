//! Built-in `date` implementation.
//!
//! Supports the subset needed by shell conformance tests: `date "+%s"`.

use crate::BuiltinResult;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn date(args: &[String], _stdin: &[u8]) -> BuiltinResult {
    if args.is_empty() || (args.len() == 1 && args[0] == "+%s") {
        let seconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_secs(),
            Err(_) => 0,
        };
        return BuiltinResult::success(format!("{seconds}\n").into_bytes());
    }

    BuiltinResult::failure(
        1,
        format!("date: unsupported arguments: {}\n", args.join(" ")).into_bytes(),
    )
}

