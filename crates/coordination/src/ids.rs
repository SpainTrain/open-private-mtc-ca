//! Domain newtypes for the lease/epoch protocol (spec §22.1).
//!
//! The fencing counter [`Epoch`](mtc::Epoch) is reused from the `mtc` core
//! crate rather than redefined here: the lease's epoch, the index counter's
//! epoch, and the checkpoint/batch epochs are all the *same* write-path
//! fencing token (spec §11), so they must be one compile-time type. This
//! module adds the checked, strictly-monotonic arithmetic the protocol needs
//! ([`EpochExt`]) and the two identifiers unique to coordination state
//! ([`HolderId`], [`Region`]).

use std::fmt;

use mtc::Epoch;

/// The epoch a freshly [`acquire`](crate::LeaseCoordinator::acquire)d lease
/// starts at (spec §8.3).
///
/// Epoch `0` is reserved as the "no primary has ever held the lease" sentinel;
/// the first successful acquire records epoch `1`, and every takeover advances
/// it by one via [`EpochExt::checked_next`]. The exact origin is an
/// implementation choice the spec does not pin — see the crate-level docs.
pub const INITIAL_EPOCH: Epoch = Epoch(1);

/// Returned by [`EpochExt::checked_next`] when an [`Epoch`] cannot advance
/// because it already holds `u64::MAX`.
///
/// This is unreachable in practice (advancing the epoch once per takeover, a
/// takeover every few seconds would take ~10^11 years to exhaust `u64`), but
/// the protocol never `unwrap`s the increment (rule no-unwrap-in-prod): the
/// overflow is a typed, propagated error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("epoch overflow: {current} is the maximum epoch and cannot be advanced")]
pub struct EpochOverflow {
    /// The epoch value that could not be advanced (always `u64::MAX`).
    pub current: u64,
}

/// Checked, strictly-monotonic arithmetic on the fencing [`Epoch`].
///
/// An extension trait rather than inherent methods because [`Epoch`] is
/// defined in the `mtc` crate (Rust's orphan rule forbids adding inherent
/// methods from here). The single operation the protocol needs is "advance to
/// the next epoch", and it is deliberately fallible: epochs only ever move
/// forward, so the sole failure mode is `u64` overflow.
pub trait EpochExt: Copy {
    /// Returns the strictly greater successor epoch, or [`EpochOverflow`] if
    /// `self` is already `u64::MAX`.
    ///
    /// This is the *only* way the protocol advances an epoch, which is what
    /// makes the sequence strictly monotonic: every takeover calls
    /// `checked_next` exactly once, so successive current epochs satisfy
    /// `prev < next` (spec §8.3 "every takeover atomically increments epoch").
    ///
    /// # Errors
    ///
    /// Returns [`EpochOverflow`] iff `self` holds `u64::MAX`.
    fn checked_next(self) -> Result<Epoch, EpochOverflow>;
}

impl EpochExt for Epoch {
    fn checked_next(self) -> Result<Epoch, EpochOverflow> {
        self.0
            .checked_add(1)
            .map(Epoch)
            .ok_or(EpochOverflow { current: self.0 })
    }
}

/// Identity of the process/region instance that holds (or seeks) the primary
/// lease — the `holder_id` attribute of the §8.2 lease item.
///
/// A newtype, never a bare `String` (rule use-newtypes): the holder id is the
/// fencing check `renew` conditions on, so it must not be interchangeable with
/// arbitrary strings. Distinct from [`Region`]: many holders may share a
/// region, but each holder id is unique to one primary instance.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HolderId(String);

impl HolderId {
    /// Wraps a holder-identity string (e.g. a region-qualified instance id).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrows the holder id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the newtype, returning the underlying `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for HolderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The cloud region a lease holder runs in — the `region` attribute of the
/// §8.2 lease item.
///
/// A newtype (rule use-newtypes): the region names *where* the current primary
/// is, which standby regions read to decide failover targets (spec §8.3).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Region(String);

impl Region {
    /// Wraps a region name (e.g. `us-east-1`).
    #[must_use]
    pub fn new(region: impl Into<String>) -> Self {
        Self(region.into())
    }

    /// Borrows the region name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the newtype, returning the underlying `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Epoch, EpochExt, EpochOverflow, HolderId, Region, INITIAL_EPOCH};

    #[test]
    fn checked_next_advances_by_one() {
        assert_eq!(Epoch(0).checked_next(), Ok(Epoch(1)));
        assert_eq!(INITIAL_EPOCH.checked_next(), Ok(Epoch(2)));
        assert_eq!(Epoch(41).checked_next(), Ok(Epoch(42)));
    }

    #[test]
    fn checked_next_is_strictly_monotonic() {
        let mut epoch = INITIAL_EPOCH;
        for _ in 0..1_000 {
            let next = epoch.checked_next().expect("no overflow near the origin");
            assert!(next.0 > epoch.0, "epoch must strictly increase");
            epoch = next;
        }
    }

    #[test]
    fn checked_next_overflows_at_u64_max() {
        assert_eq!(
            Epoch(u64::MAX).checked_next(),
            Err(EpochOverflow { current: u64::MAX })
        );
    }

    #[test]
    fn holder_id_round_trips() {
        let holder = HolderId::new("us-east-1/instance-7");
        assert_eq!(holder.as_str(), "us-east-1/instance-7");
        assert_eq!(holder.to_string(), "us-east-1/instance-7");
        assert_eq!(holder.clone().into_string(), "us-east-1/instance-7");
        assert_eq!(holder, HolderId::new("us-east-1/instance-7".to_string()));
    }

    #[test]
    fn region_round_trips() {
        let region = Region::new("eu-west-1");
        assert_eq!(region.as_str(), "eu-west-1");
        assert_eq!(region.to_string(), "eu-west-1");
        assert_eq!(region.into_string(), "eu-west-1");
    }
}
