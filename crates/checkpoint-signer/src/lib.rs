//! Checkpoint signer — **write-path step 7** (spec §11.1): build a checkpoint,
//! sign it on the HSM, and frame the §8.1 signed-checkpoint object.
//!
//! The CA periodically commits "the log has exactly `tree_size` entries and
//! their Merkle root is `root_hash`" by signing a checkpoint (spec §2). This
//! crate is the step that turns a computed `(tree_size, root_hash)` into signed
//! bytes ready for the step-8 commit (spec §11.1): it drives mtc's typestate
//! [`CheckpointBuilder`](mtc::CheckpointBuilder), signs the canonical
//! domain-separated `MTCSubtreeSignatureInput` on the injected
//! [`Hsm`](cloud_types::Hsm), and frames the object bytes.
//!
//! # Self-contained framing (dispatch "Option B")
//!
//! `crates/mtc` deliberately exposes **no public seam to attach an externally
//! produced (HSM) signature** to a [`Checkpoint`](mtc::Checkpoint): its
//! [`sign`](mtc::Checkpoint::sign) is the software path and `with_signature` is
//! test-only. Rather than widen mtc's surface here (a typed `into_signed` seam
//! is the separate fast-follow bead `mtc-qka.12`), this crate:
//!
//! 1. builds a [`Checkpoint<Unsigned>`](mtc::Checkpoint) via
//!    [`CheckpointBuilder`](mtc::CheckpointBuilder),
//! 2. gets the exact bytes to sign from
//!    [`Checkpoint::signature_input`](mtc::Checkpoint::signature_input) — it
//!    never re-implements the `mtc-subtree/v1` domain label or the
//!    canonicalization (draft-ietf-plants-merkle-tree-certs-03 §5.4.1;
//!    ADR-0005),
//! 3. HSM-signs those bytes (ECDSA P-256, 64-byte `r || s` IEEE P1363;
//!    ADR-0003), and
//! 4. frames the §8.1 object body itself, **byte-identical** to mtc's
//!    TLS-presentation signed-checkpoint format.
//!
//! Point 4 is guarded against drift by an oracle test: the framed bytes must
//! parse back through [`mtc::Checkpoint::parse_tls_presentation`], round-trip to
//! the same fields and signature, and re-serialize to the identical bytes (see
//! [`framing`]).
//!
//! # What crosses the HSM boundary (spec §14, ADR-0003)
//!
//! The private key never leaves the HSM: this crate hands the HSM the bytes to
//! sign and receives a 64-byte P1363 `r || s` signature, consumed as-is (high-`s`
//! permitted — production HSM ECDSA is *randomized*, so signatures are not
//! canonical and **must never** key storage; ADR-0003 B.1). Idempotency of
//! checkpoint publication therefore comes from the object **key**, addressed by
//! `tree_size` — `checkpoints/{tree_size:016}.signed` — not by signature bytes
//! (ADR-0003 B.2; see [`framing::checkpoint_object_key`]).
//!
//! # Retry / backoff (spec §11.3 row 7)
//!
//! A *transient* HSM failure (`CloudError::Transport { retryable: true }`) is
//! retried with exponential backoff timed by the injected
//! [`Clock`](clock::Clock) (rule `no-systemtime-now-in-prod`; never
//! `SystemTime::now` or `tokio::time` directly). A terminal failure, or
//! transient failures that outlast the retry budget, surface as the distinct,
//! alert-worthy [`CheckpointSignError::HsmSigningFailed`] ("alert if
//! persistent").
//!
//! # Scope
//!
//! v1 ECDSA P-256 only (spec §14.1). Out of scope: ML-DSA (v2), `CloudHSM`
//! production keys / rotation, and the step-8 commit of the signed checkpoint.

mod error;
mod framing;
mod retry;
mod signer;

/// The fixed byte length of an ECDSA P-256 `r || s` IEEE P1363 signature
/// (ADR-0003): 32-byte big-endian `r` followed by 32-byte big-endian `s`.
pub(crate) const P1363_SIGNATURE_LEN: usize = 64;

pub use error::CheckpointSignError;
pub use framing::{checkpoint_object_key, SignedCheckpointObject, CHECKPOINT_OBJECT_PREFIX};
pub use retry::RetryPolicy;
pub use signer::CheckpointSigner;
