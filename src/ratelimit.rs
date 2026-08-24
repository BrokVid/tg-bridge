use std::collections::HashMap;
use std::sync::Mutex;

/// Fixed-window per-client rate limiter (window = 1 minute of unix time).
#[derive(Default)]
pub struct RateLimiter {
    rpm: u32,
    buckets: Mutex<HashMap<String, (u64, u32)>>,
}

impl RateLimiter {
    pub fn new(rpm: u32) -> Self {
        Self {
            rpm,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns false when the client exceeded its quota for the current window.
    pub fn allow(&self, client: &str) -> bool {
        let now_min: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            / 60;
        let mut b = self.buckets.lock().expect("poisoned");
        let entry = b.entry(client.to_owned()).or_insert((now_min, 0));
        if entry.0 != now_min {
            *entry = (now_min, 0);
        }
        entry.1 += 1;
        entry.1 <= self.rpm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_quota_and_blocks_after() {
        let rl = RateLimiter::new(3);
        assert!(rl.allow("c"));
        assert!(rl.allow("c"));
        assert!(rl.allow("c"));
        assert!(!rl.allow("c"));
        assert!(rl.allow("other"));
    }
}
