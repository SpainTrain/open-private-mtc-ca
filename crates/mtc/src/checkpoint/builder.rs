//! The typestate builder for [`Checkpoint`] (spec §22.2, "Builder pattern via
//! typestate").
//!
//! [`CheckpointBuilder`] proves *at compile time* that every required field —
//! `log_id`, `root_hash`, `tree_size`, `signed_at` — is present before
//! [`build`](CheckpointBuilder::build) can be called. The three optional-until-
//! set fields are tracked by type parameters that flip from a `No…` marker to a
//! `With…` marker as each setter runs; `build` is implemented **only** for the
//! fully-populated combination, so calling it on an incomplete builder is a
//! *compile error*, not a runtime one (spec §22.2: "The agent literally cannot
//! write code that constructs a partial `Checkpoint`"). See
//! `tests/compile_fail/checkpoint_build_incomplete.rs` for the proof.
//!
//! The setters are generic over the *other two* markers, so fields may be set
//! in any order; each can be set exactly once (a `With…`-state builder has no
//! second setter for that field).

use core::marker::PhantomData;

use crate::types::{HashOutput, LogId, TreeSize};

use super::{Checkpoint, SignedAt, Unsigned};

/// The `root_hash` field is not yet set.
#[derive(Debug)]
pub struct NoRootHash;
/// The `root_hash` field is set to the contained value.
#[derive(Debug)]
pub struct WithRootHash(HashOutput);

/// The `tree_size` field is not yet set.
#[derive(Debug)]
pub struct NoTreeSize;
/// The `tree_size` field is set to the contained value.
#[derive(Debug)]
pub struct WithTreeSize(TreeSize);

/// The `signed_at` field is not yet set.
#[derive(Debug)]
pub struct NoSignedAt;
/// The `signed_at` field is set to the contained value.
#[derive(Debug)]
pub struct WithSignedAt(SignedAt);

/// A compile-time-checked builder for an [`Checkpoint`] (spec §22.2).
///
/// Start with [`CheckpointBuilder::new`] (which fixes the `log_id`), then call
/// [`root_hash`](Self::root_hash), [`tree_size`](Self::tree_size), and
/// [`signed_at`](Self::signed_at) in any order. Only once all three are set
/// does the type gain a [`build`](CheckpointBuilder::build) method returning an
/// unsigned [`Checkpoint`].
///
/// The `R`, `T`, `S` type parameters are the presence markers for `root_hash`,
/// `tree_size`, and `signed_at`; they default to the `No…` states so
/// `CheckpointBuilder::new(..)` names the fresh builder without spelling them
/// out.
#[derive(Debug)]
pub struct CheckpointBuilder<R = NoRootHash, T = NoTreeSize, S = NoSignedAt> {
    log_id: LogId,
    root_hash: R,
    tree_size: T,
    signed_at: S,
    _marker: PhantomData<(R, T, S)>,
}

impl CheckpointBuilder {
    /// Starts a builder for a checkpoint of the log identified by `log_id`.
    ///
    /// The returned builder has no `root_hash`, `tree_size`, or `signed_at`
    /// yet, so it has no `build` method until all three are supplied.
    #[must_use]
    pub const fn new(log_id: LogId) -> Self {
        Self {
            log_id,
            root_hash: NoRootHash,
            tree_size: NoTreeSize,
            signed_at: NoSignedAt,
            _marker: PhantomData,
        }
    }
}

impl<T, S> CheckpointBuilder<NoRootHash, T, S> {
    /// Sets the committed Merkle root hash (spec §2, "Checkpoint").
    ///
    /// Available only while `root_hash` is unset, so it cannot be set twice.
    #[must_use]
    pub fn root_hash(self, root_hash: HashOutput) -> CheckpointBuilder<WithRootHash, T, S> {
        CheckpointBuilder {
            log_id: self.log_id,
            root_hash: WithRootHash(root_hash),
            tree_size: self.tree_size,
            signed_at: self.signed_at,
            _marker: PhantomData,
        }
    }
}

impl<R, S> CheckpointBuilder<R, NoTreeSize, S> {
    /// Sets the committed tree size — the number of entries the checkpoint
    /// commits to (spec §2, "Checkpoint").
    ///
    /// Available only while `tree_size` is unset, so it cannot be set twice.
    #[must_use]
    pub fn tree_size(self, tree_size: TreeSize) -> CheckpointBuilder<R, WithTreeSize, S> {
        CheckpointBuilder {
            log_id: self.log_id,
            root_hash: self.root_hash,
            tree_size: WithTreeSize(tree_size),
            signed_at: self.signed_at,
            _marker: PhantomData,
        }
    }
}

impl<R, T> CheckpointBuilder<R, T, NoSignedAt> {
    /// Sets the checkpoint's timestamp (spec §8.2 `signed_at`; supplied from
    /// the injected `Clock` by the write path, never from a direct
    /// `SystemTime::now`, rule `no-systemtime-now-in-prod`).
    ///
    /// Available only while `signed_at` is unset, so it cannot be set twice.
    /// Note: `signed_at` is checkpoint *metadata* and is **not** part of the
    /// signed payload (draft §5.4.1 carries no timestamp) — see
    /// [`Checkpoint::signature_input`](super::Checkpoint::signature_input).
    #[must_use]
    pub fn signed_at(self, signed_at: SignedAt) -> CheckpointBuilder<R, T, WithSignedAt> {
        CheckpointBuilder {
            log_id: self.log_id,
            root_hash: self.root_hash,
            tree_size: self.tree_size,
            signed_at: WithSignedAt(signed_at),
            _marker: PhantomData,
        }
    }
}

// The one reachable `build`: only a fully-populated builder has it. Calling
// `.build()` on any other state is a compile error (spec §22.2).
impl CheckpointBuilder<WithRootHash, WithTreeSize, WithSignedAt> {
    /// Finalizes an **unsigned** [`Checkpoint`].
    ///
    /// Reachable only once `log_id`, `root_hash`, `tree_size`, and `signed_at`
    /// are all set (spec §22.2). The result is a [`Checkpoint<Unsigned>`]: it
    /// carries no signature and cannot be verified until
    /// [`sign`](Checkpoint::sign) transitions it to
    /// [`Checkpoint<Signed>`](super::Signed) (spec §22.4).
    #[must_use]
    pub fn build(self) -> Checkpoint<Unsigned> {
        Checkpoint {
            log_id: self.log_id,
            tree_size: self.tree_size.0,
            root_hash: self.root_hash.0,
            signed_at: self.signed_at.0,
            state: Unsigned,
        }
    }
}
