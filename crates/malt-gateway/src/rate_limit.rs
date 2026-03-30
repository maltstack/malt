// Rate limiting middleware — per-client token bucket.

use std::collections::HashMap;
use std::sync::Mutex;

/// Simple per-client rate limiter with a fixed token count per window.
pub struct RateLimiter {
    max_per_window: usize,
    buckets: Mutex<HashMap<String, usize>>,
}

impl RateLimiter {
    /// Create a new rate limiter allowing `max_per_window` requests per client per window.
    pub fn new(max_per_window: usize) -> Self {
        Self {
            max_per_window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Consume one token for `client_id`. Returns `false` if the budget is exhausted.
    pub fn check(&self, client_id: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let used = buckets.entry(client_id.to_owned()).or_insert(0);
        if *used >= self.max_per_window {
            return false;
        }
        *used += 1;
        true
    }

    /// Reset the token count for a single client.
    pub fn refill(&self, client_id: &str) {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets.remove(client_id);
    }

    /// Reset all clients' token counts.
    pub fn refill_all(&self) {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        buckets.clear();
    }
}
