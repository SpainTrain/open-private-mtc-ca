//! Deterministic, seeded randomness for fixture construction (spec §19.2,
//! §19.3 Layer 1).
//!
//! Property tests get their randomness from `proptest`'s own `Strategy`
//! machinery (see [`crate::strategy`]); this module is for the *other* half —
//! plain fixture-construction code that is not itself a `proptest!` body but
//! still wants reproducible pseudo-randomness, e.g. building a batch of leaf
//! entries for a benchmark or a one-off regression fixture (see
//! [`crate::fixtures`]). The seed is the entire source of entropy: the same
//! seed always produces the same output sequence, so a fixture built from
//! `seeded_rng(42)` is repeatable across runs and machines.
//!
//! This is deliberately unrelated to cryptographic key generation:
//! `EcdsaP256::generate_keypair` (spec §14.1) draws from the OS RNG and is not
//! seedable, so fixtures that need a signature still produce a fresh keypair
//! per call — only the *content* fields are deterministic (see
//! [`crate::fixtures::signed_checkpoint`]).

pub use rand::{rngs::StdRng, Rng, RngCore, SeedableRng};

/// Creates a deterministic pseudo-random generator from `seed`.
///
/// The same `seed` always produces the same output sequence (for a fixed
/// `rand` dependency version — [`StdRng`]'s own documentation notes that the
/// concrete algorithm, and so the exact sequence, is not guaranteed stable
/// *across* `rand` versions). Use this instead of `rand::thread_rng()` or
/// `OsRng` anywhere a fixture must be reproducible across test runs.
///
/// # Examples
///
/// ```
/// use mtc_testutil::rng::{seeded_rng, RngCore};
///
/// let mut a = seeded_rng(42);
/// let mut b = seeded_rng(42);
/// assert_eq!(a.next_u64(), b.next_u64());
/// ```
#[must_use]
pub fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

#[cfg(test)]
mod tests {
    use super::seeded_rng;
    use rand::RngCore;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = seeded_rng(1234);
        let mut b = seeded_rng(1234);
        let seq_a: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_eq!(seq_a, seq_b, "same seed must produce the same sequence");
    }

    #[test]
    fn different_seeds_diverge() {
        // Not a formal guarantee, but a same-output collision on 8 u64s from
        // two distinct seeds would be an astronomically unlikely PRNG defect,
        // so this is a reasonable sanity check that the seed is actually used.
        let mut a = seeded_rng(1);
        let mut b = seeded_rng(2);
        let seq_a: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn repeated_calls_with_the_same_seed_are_independent_generators() {
        // seeded_rng(seed) always starts the sequence over; advancing one
        // instance must not affect a freshly created one from the same seed.
        let mut first = seeded_rng(99);
        let _ = first.next_u64();
        let _ = first.next_u64();
        let mut fresh = seeded_rng(99);
        assert_eq!(fresh.next_u64(), seeded_rng(99).next_u64());
    }
}
