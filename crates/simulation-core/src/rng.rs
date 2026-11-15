use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};

/// Version-pinned deterministic pseudo-random number generator backed by ChaCha8.
#[derive(Debug, Clone)]
pub struct DeterministicRng {
    seed: u64,
    step_count: u64,
    inner: ChaCha8Rng,
}

impl DeterministicRng {
    /// Creates a new generator from a 64-bit seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            step_count: 0,
            inner: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// Gets the original seed used to initialize the generator.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the total count of random values generated.
    #[must_use]
    pub const fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Generates the next deterministic 32-bit unsigned integer.
    pub fn next_u32(&mut self) -> u32 {
        self.step_count += 1;
        self.inner.next_u32()
    }

    /// Generates the next deterministic 64-bit unsigned integer.
    pub fn next_u64(&mut self) -> u64 {
        self.step_count += 1;
        self.inner.next_u64()
    }

    /// Generates a deterministic integer in `[min, max]` inclusive.
    ///
    /// # Panics
    /// Panics if `min > max`.
    pub fn gen_range_inclusive(&mut self, min: i32, max: i32) -> i32 {
        assert!(min <= max, "min must be <= max");
        if min == max {
            return min;
        }
        let span = (i64::from(max) - i64::from(min) + 1) as u64;
        let rand_val = self.next_u64();
        let offset = (rand_val % span) as i32;
        min + offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_seed_produces_identical_sequence() {
        let mut rng1 = DeterministicRng::new(42);
        let mut rng2 = DeterministicRng::new(42);

        for _ in 0..10_000 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
        assert_eq!(rng1.step_count(), 10_000);
        assert_eq!(rng2.step_count(), 10_000);
    }

    #[test]
    fn test_different_seed_produces_different_sequence() {
        let mut rng1 = DeterministicRng::new(42);
        let mut rng2 = DeterministicRng::new(999);

        assert_ne!(rng1.next_u64(), rng2.next_u64());
    }

    #[test]
    fn test_gen_range_inclusive() {
        let mut rng = DeterministicRng::new(12345);
        for _ in 0..1_000 {
            let v = rng.gen_range_inclusive(-10, 10);
            assert!((-10..=10).contains(&v));
        }
    }
}
