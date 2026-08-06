//! Basic no-panic Kani harnesses for the lease/epoch primitives (rule
//! kani-for-critical-paths, spec §19.12).
//!
//! Scope is deliberately narrow: these prove the pure, synchronous helpers are
//! panic-free (and a couple of soundness lower bounds) over their full input
//! space. The *full* lease/epoch invariant proofs — no two regions hold a
//! current-epoch lease, takeover atomicity — are a separate bead (mtc-8l0u).
//!
//! Compiled only under `cargo kani` (the module is `#[cfg(kani)]` in `lib.rs`),
//! so it never affects `cargo build`/`test`/`clippy`.

use cloud_types::{Condition, Value};

use crate::protocol::takeover_eligible_millis;
use crate::{epoch_condition, Epoch, EpochExt};

/// Advancing any epoch never panics; on success it is strictly greater by one,
/// and it fails only at `u64::MAX` (spec §8.3 monotonic epoch).
#[kani::proof]
fn epoch_checked_next_is_monotonic_and_total() {
    let raw: u64 = kani::any();
    match Epoch(raw).checked_next() {
        Ok(next) => {
            assert!(raw < u64::MAX);
            assert!(next.0 > raw); // strictly monotonic
        }
        Err(overflow) => {
            assert!(raw == u64::MAX);
            assert!(overflow.current == u64::MAX);
        }
    }
}

/// The takeover-eligibility test never panics for any inputs (its arithmetic
/// saturates), and it is never eligible before the lease's own expiry — a
/// challenger cannot take over a lease that has not even reached `expires_at`.
#[kani::proof]
fn takeover_eligibility_is_total_and_sound() {
    let now: u64 = kani::any();
    let expires_at: u64 = kani::any();
    let margin: u64 = kani::any();

    let eligible = takeover_eligible_millis(now, expires_at, margin);

    if now < expires_at {
        assert!(!eligible);
    }
}

/// Building the fencing condition never panics and always targets the given
/// epoch value (spec §8.3 epoch condition).
#[kani::proof]
fn epoch_condition_is_total() {
    let raw: u64 = kani::any();
    match epoch_condition(Epoch(raw)) {
        Condition::AttributeEquals { expected, .. } => {
            assert!(expected == Value::U64(raw));
        }
        _ => assert!(false, "epoch_condition must be AttributeEquals"),
    }
}
