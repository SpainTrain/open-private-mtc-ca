//! [`LeafBytes`]: the typed, framing-checked bytes the Merkle tree ingests as a
//! leaf — the write/read seam that keeps the certificate chain sound.
//!
//! # Why this type exists (crypto audit 2026-08-05, Finding 2)
//!
//! Soundness of the whole chain (leaf hash → inclusion proof → checkpoint
//! signature) depends on the CA's **write path** committing exactly the bytes a
//! relying party **reconstructs** on the read path. A relying party reconstructs
//! a leaf as `HASH(0x00 || LogEntry::tls_serialize)` — the domain-separated
//! [`hash_leaf`](crate::hash_leaf) over a serialized [`LogEntry`](crate::LogEntry)
//! **with its `00 00` / `00 01…` entry-type discriminant** (spec §19.2;
//! draft-ietf-plants-merkle-tree-certs-03 §5.3, §7.2).
//!
//! [`MerkleTree::append`](crate::MerkleTree::append) previously took raw
//! `&[u8]`, so nothing stopped a caller from appending, say, the bare TBS bytes
//! (dropping the discriminant frame). The audit showed that mistake yields a
//! tree that **silently** fails relying-party verification for every entry — a
//! caller contract with no enforcement.
//!
//! `LeafBytes` closes that trap by construction. Its inner bytes are private and
//! there is **no public constructor from arbitrary bytes**: the sole public
//! producer is [`LogEntry::leaf_bytes`](crate::LogEntry::leaf_bytes), which runs
//! the *same* serialization [`LogEntry::leaf_hash`](crate::LogEntry::leaf_hash)
//! uses. So a value of this type can only be the correctly-framed serialization
//! of a real log entry, and appending un-framed bytes is not a runtime hazard —
//! it does not typecheck (`tests/compile_fail/append_raw_bytes.rs`).
//!
//! The tree stays decoupled from the entry layer: it ingests `&LeafBytes` and
//! never names [`LogEntry`](crate::LogEntry). This module is owned by neither —
//! it is the narrow seam between entry framing and tree ingestion.

/// The exact bytes committed as one Merkle-tree leaf.
///
/// This is a serialized [`LogEntry`](crate::LogEntry) (entry-type discriminant
/// first) — the leaf *preimage* the domain-separated
/// [`hash_leaf`](crate::hash_leaf) is taken over.
///
/// The inner buffer is private. There is no way to build a `LeafBytes` from
/// arbitrary bytes outside this crate; the only public producer is
/// [`LogEntry::leaf_bytes`](crate::LogEntry::leaf_bytes) (so, for the gap
/// placeholder, `null_entry().leaf_bytes()`). This is what makes the CA's
/// committed-leaf framing the *same* code path a relying party reconstructs and
/// verifies against — see the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeafBytes(Vec<u8>);

impl LeafBytes {
    /// Wraps already-framed leaf bytes.
    ///
    /// Crate-internal on purpose: the sanctioned public entry point is
    /// [`LogEntry::leaf_bytes`](crate::LogEntry::leaf_bytes), which passes the
    /// serialization of a real [`LogEntry`](crate::LogEntry) (discriminant
    /// included). Keeping this `pub(crate)` is what makes raw-byte leaves
    /// unconstructable outside the sanctioned framing path.
    pub(crate) const fn from_framed(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The framed leaf preimage: `type || body`, ready to be
    /// [`hash_leaf`](crate::hash_leaf)ed as `HASH(0x00 || type || body)`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
