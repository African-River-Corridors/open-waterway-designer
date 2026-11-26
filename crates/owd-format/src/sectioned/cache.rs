//! A least-recently-used cache of decoded blocks with a memory budget in bytes (COG-1).

use std::collections::HashMap;
use std::hash::Hash;

/// Something the cache holds, and its size in bytes.
pub trait Weigh {
    fn bytes(&self) -> u64;
}

/// What the cache has done: for tests, and for the numbers a surface shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Bytes held now, and the most ever held at once.
    pub bytes: u64,
    pub peak_bytes: u64,
}

/// The cache. A value larger than the whole budget is still kept (alone), so a read never fails
/// for want of room; everything older goes first.
#[derive(Debug)]
pub struct Lru<K, V> {
    budget: u64,
    clock: u64,
    map: HashMap<K, (u64, V)>,
    stats: CacheStats,
}

impl<K: Hash + Eq + Copy, V: Weigh + Clone> Lru<K, V> {
    pub fn new(budget: u64) -> Lru<K, V> {
        Lru {
            budget,
            clock: 0,
            map: HashMap::new(),
            stats: CacheStats::default(),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Set a new budget; blocks over it go at once.
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.trim(0);
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    pub fn reset_peak(&mut self) {
        self.stats.peak_bytes = self.stats.bytes;
    }

    pub fn get(&mut self, k: &K) -> Option<V> {
        self.clock += 1;
        let now = self.clock;
        match self.map.get_mut(k) {
            Some((t, v)) => {
                *t = now;
                self.stats.hits += 1;
                Some(v.clone())
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Drop the oldest blocks until `extra` more bytes fit.
    fn trim(&mut self, extra: u64) {
        while self.stats.bytes + extra > self.budget && !self.map.is_empty() {
            let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some((_, v)) = self.map.remove(&oldest) {
                self.stats.bytes -= v.bytes();
                self.stats.evictions += 1;
            }
        }
    }

    pub fn put(&mut self, k: K, v: V) {
        self.clock += 1;
        if let Some((_, old)) = self.map.remove(&k) {
            self.stats.bytes -= old.bytes();
        }
        let n = v.bytes();
        self.trim(n);
        self.stats.bytes += n;
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.stats.bytes);
        self.map.insert(k, (self.clock, v));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct B(u64);
    impl Weigh for B {
        fn bytes(&self) -> u64 {
            self.0
        }
    }

    #[test]
    fn the_cache_keeps_to_its_budget_and_drops_the_oldest() {
        let mut c: Lru<u32, B> = Lru::new(100);
        for k in 0..10 {
            c.put(k, B(30));
            assert!(c.stats().bytes <= 100);
        }
        assert_eq!(c.len(), 3);
        assert!(c.get(&9).is_some() && c.get(&0).is_none());
        c.get(&7);
        c.put(10, B(30));
        assert!(c.get(&7).is_some(), "a block read lately stays");
        assert!(c.get(&8).is_none(), "the oldest goes");
        assert_eq!(c.stats().peak_bytes, 90);
    }
}
