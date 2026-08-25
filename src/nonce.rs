//! In-memory replay protection: a (client, signature) pair seen within the
//! TTL is rejected as a replay. Signature is unique per (timestamp, body),
//! so a legit retry always re-signs with a fresh timestamp and passes.
//!
//! State is in-memory and intentionally not persisted: after a restart the
//! timestamp window (±60s by default) still bounds the replay exposure.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

const MAX_ENTRIES: usize = 100_000;

pub struct NonceCache {
    inner: Mutex<Inner>,
    ttl_secs: i64,
}

struct Inner {
    /// key -> expiry (unix secs)
    seen: HashMap<String, i64>,
    last_sweep: i64,
}

impl NonceCache {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                seen: HashMap::new(),
                last_sweep: 0,
            }),
            ttl_secs,
        }
    }

    /// Returns `true` when the key is new within the TTL window and records
    /// it; returns `false` when it was already seen (replay).
    pub fn insert_if_absent(&self, key: &str, now: i64) -> bool {
        let mut g = self.lock();
        match g.seen.get(key) {
            Some(&exp) if exp > now => return false,
            _ => {}
        }
        g.seen.insert(key.to_owned(), now + self.ttl_secs);
        self.sweep_if_needed(&mut g, now);
        true
    }

    /// Bounds memory: sweep on size pressure or at most once per TTL. If the
    /// map is still full after dropping expired keys (flood of distinct valid
    /// requests), evict an arbitrary entry rather than grow without limit.
    fn sweep_if_needed(&self, g: &mut Inner, now: i64) {
        if g.seen.len() < MAX_ENTRIES && now - g.last_sweep < self.ttl_secs {
            return;
        }
        g.seen.retain(|_, exp| *exp > now);
        if g.seen.len() >= MAX_ENTRIES {
            if let Some(k) = g.seen.keys().next().cloned() {
                g.seen.remove(&k);
            }
        }
        g.last_sweep = now;
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock must not take the whole bridge down;
        // the cache is reconstructible state.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_seen_true_duplicate_false() {
        let c = NonceCache::new(60);
        assert!(c.insert_if_absent("a", 1000));
        assert!(!c.insert_if_absent("a", 1001));
        assert!(c.insert_if_absent("b", 1001));
    }

    #[test]
    fn key_expires_after_ttl() {
        let c = NonceCache::new(60);
        assert!(c.insert_if_absent("a", 1000));
        assert!(!c.insert_if_absent("a", 1050));
        assert!(c.insert_if_absent("b", 1061));
        assert!(c.insert_if_absent("a", 1062), "expired entry must be reusable");
    }

    #[test]
    fn sweep_bounds_memory() {
        let c = NonceCache::new(60);
        for i in 0..(MAX_ENTRIES + 5000) as i64 {
            let _ = c.insert_if_absent(&format!("k{i}"), 1000 + i / 10_000);
        }
        let g = c.lock();
        assert!(g.seen.len() <= MAX_ENTRIES);
        assert!(!g.seen.is_empty());
    }
}
