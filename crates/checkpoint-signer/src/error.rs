//! The library error type for checkpoint signing (rule
//! `thiserror-for-libs-eyre-for-bins`, spec §22.6): failure modes live in the
//! signature as an enum a caller can match on, not an opaque report.

use cloud_types::CloudError;
use mtc::TrustAnchorIdError;

/// A checkpoint signing operation failed.
///
/// The enum keeps the write path's failure modes explicit (spec §22.6): the
/// step-8 caller can match [`HsmSigningFailed`](Self::HsmSigningFailed) to fire
/// the §11.3-row-7 "alert if persistent" signal, and the two byte-shape faults
/// ([`Input`](Self::Input), [`MalformedSignature`](Self::MalformedSignature))
/// distinguish a bad log id from a broken HSM.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointSignError {
    /// The canonical `MTCSubtreeSignatureInput` could not be assembled: the
    /// log's `TrustAnchorID` is not 1..=255 bytes
    /// (draft-ietf-plants-merkle-tree-certs-03 §5.4.1 `opaque<1..2^8-1>`).
    #[error("cannot assemble the checkpoint signature input: {0}")]
    Input(#[from] TrustAnchorIdError),

    /// The HSM returned a signature that was not the fixed 64-byte P1363
    /// `r || s` the [`Hsm::sign`](cloud_types::Hsm::sign) contract mandates
    /// (ADR-0003). Alert-worthy: it means a wrong-curve key or a broken token,
    /// and it is never retried (a retry cannot change the encoding).
    #[error(
        "HSM returned a {actual}-byte signature; expected a {expected}-byte P1363 r‖s signature \
         (ADR-0003)"
    )]
    MalformedSignature {
        /// The required length (64 bytes).
        expected: usize,
        /// The length the HSM actually returned.
        actual: usize,
    },

    /// Persistent HSM signing failure (spec §11.3 row 7). Either a terminal
    /// (non-retryable) error, or transient errors that outlasted the retry
    /// budget.
    ///
    /// This is the **distinct, alert-worthy** variant: the write path cannot
    /// commit a checkpoint, so it surfaces to the operator / alerting path
    /// (runbook `mtc-16vs`, "HSM unavailability"). The embedded [`CloudError`]
    /// preserves the underlying cause (a `NotFound` bad key handle reads
    /// differently from a `Transport` outage).
    #[error("HSM checkpoint signing failed after {attempts} attempt(s): {source}")]
    HsmSigningFailed {
        /// Total number of HSM `sign` calls made (the initial try plus every
        /// retry).
        attempts: u32,
        /// The last HSM error observed.
        #[source]
        source: CloudError,
    },
}
