//! Anti-replay nonces (RFC 8555 §6.5).
//!
//! Nonces are single-use: issuing registers them, and the first `consume`
//! removes them, so a replayed nonce (or one the server never issued) fails
//! with `badNonce`. Expiry is driven by the injected [`Clock`] so tests can
//! advance time deterministically. Memory is bounded by issue-rate × TTL:
//! expired entries are pruned on every issue.

use std::collections::HashMap;
use std::fmt;

use base64ct::{Base64UrlUnpadded, Encoding};

use crate::clock::{Clock, MonotonicMillis};
use crate::error::AcmeError;

/// Default nonce lifetime: 5 minutes.
pub const DEFAULT_NONCE_TTL_MILLIS: u64 = 5 * 60 * 1000;

/// Entropy per nonce (128 bits, base64url-encoded to 22 characters).
const NONCE_BYTES: usize = 16;

/// An anti-replay nonce value (base64url token, newtype per `use-newtypes`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Nonce(String);

impl Nonce {
    /// Wraps a client-presented nonce value (validity is decided by
    /// [`NonceStore::consume`], not by parsing).
    #[must_use]
    pub fn from_client(value: &str) -> Self {
        Self(value.to_owned())
    }

    /// The nonce as it appears in `Replay-Nonce` headers.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// In-memory single-use nonce registry.
///
/// Persistence is deliberately out of scope for this ticket; the store lives
/// behind a lock in [`crate::routes::AcmeState`].
#[derive(Debug)]
pub struct NonceStore {
    ttl_millis: u64,
    /// nonce -> expiry deadline.
    issued: HashMap<Nonce, MonotonicMillis>,
}

impl NonceStore {
    /// Creates a store whose nonces live for `ttl_millis`.
    #[must_use]
    pub fn new(ttl_millis: u64) -> Self {
        Self {
            ttl_millis,
            issued: HashMap::new(),
        }
    }

    /// Issues a fresh random nonce, registering it as spendable until
    /// now + TTL.
    ///
    /// # Errors
    /// [`AcmeError::Internal`] if the OS RNG fails.
    pub fn issue(&mut self, clock: &dyn Clock) -> Result<Nonce, AcmeError> {
        let now = clock.monotonic_now();
        self.prune_expired(now);
        let mut buf = [0u8; NONCE_BYTES];
        getrandom::fill(&mut buf).map_err(|e| AcmeError::Internal(format!("os rng: {e}")))?;
        let nonce = Nonce(Base64UrlUnpadded::encode_string(&buf));
        self.issued
            .insert(nonce.clone(), now.saturating_add(self.ttl_millis));
        Ok(nonce)
    }

    /// Spends `nonce`. Each issued nonce is consumable exactly once.
    ///
    /// # Errors
    /// [`AcmeError::BadNonce`] if the nonce was never issued, already
    /// consumed, or has expired.
    pub fn consume(&mut self, nonce: &Nonce, clock: &dyn Clock) -> Result<(), AcmeError> {
        let now = clock.monotonic_now();
        match self.issued.remove(nonce) {
            Some(deadline) if now <= deadline => Ok(()),
            // Expired (already removed above) or unknown: same client-visible error.
            _ => Err(AcmeError::BadNonce),
        }
    }

    /// Number of currently outstanding (unspent, unpruned) nonces.
    #[must_use]
    pub fn outstanding(&self) -> usize {
        self.issued.len()
    }

    fn prune_expired(&mut self, now: MonotonicMillis) {
        self.issued.retain(|_, deadline| now <= *deadline);
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::clock::ManualClock;

    #[test]
    fn issued_nonce_is_consumable_exactly_once() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(DEFAULT_NONCE_TTL_MILLIS);
        let nonce = store.issue(&clock).expect("issue");
        assert_eq!(store.consume(&nonce, &clock), Ok(()));
        assert_eq!(store.consume(&nonce, &clock), Err(AcmeError::BadNonce));
    }

    #[test]
    fn unknown_nonce_is_rejected() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(DEFAULT_NONCE_TTL_MILLIS);
        let forged = Nonce::from_client("AAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(store.consume(&forged, &clock), Err(AcmeError::BadNonce));
    }

    #[test]
    fn expired_nonce_is_rejected() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(1_000);
        let nonce = store.issue(&clock).expect("issue");
        clock.advance(1_001);
        assert_eq!(store.consume(&nonce, &clock), Err(AcmeError::BadNonce));
    }

    #[test]
    fn nonce_valid_up_to_its_deadline() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(1_000);
        let nonce = store.issue(&clock).expect("issue");
        clock.advance(1_000); // exactly at the deadline: still valid
        assert_eq!(store.consume(&nonce, &clock), Ok(()));
    }

    #[test]
    fn issue_prunes_expired_nonces() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(1_000);
        for _ in 0..10 {
            store.issue(&clock).expect("issue");
        }
        assert_eq!(store.outstanding(), 10);
        clock.advance(2_000);
        store.issue(&clock).expect("issue");
        assert_eq!(store.outstanding(), 1); // the 10 expired ones are gone
    }

    #[test]
    fn nonces_are_unique_and_base64url() {
        let clock = ManualClock::new();
        let mut store = NonceStore::new(DEFAULT_NONCE_TTL_MILLIS);
        let a = store.issue(&clock).expect("issue");
        let b = store.issue(&clock).expect("issue");
        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 22); // 16 bytes, unpadded base64url
        assert!(Base64UrlUnpadded::decode_vec(a.as_str()).is_ok());
    }
}
