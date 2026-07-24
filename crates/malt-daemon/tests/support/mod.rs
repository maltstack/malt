//! Deterministic helpers shared by daemon concurrency integration tests.
//!
//! These commands use MASH builtins only, so they are portable across the
//! native Windows, Linux, and macOS test targets supported by the daemon.

#![allow(dead_code)]

use std::time::{Duration, Instant};

pub const TEST_DEADLINE: Duration = Duration::from_secs(1);

/// A portable busy command. MASH's `sleep` builtin accepts integer seconds.
pub fn malt_sleep(seconds: u64) -> String {
    format!("sleep {seconds}")
}

/// Produce enough output to exercise bounded finalization without relying on
/// platform shell utilities. `printf` is supplied by MASH.
pub fn high_output(lines: usize) -> String {
    format!("i=0; while [ $i -lt {lines} ]; do printf 'malt-output-%s\\n' \"$i\"; i=$((i + 1)); done")
}

/// Poll a fallible operation until it succeeds or the explicit deadline
/// expires. The final error is returned unchanged for useful test failures.
pub fn poll_until<T, E>(deadline: Duration, mut operation: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let started = Instant::now();
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}
