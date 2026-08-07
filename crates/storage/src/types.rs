//! Supporting facade types for the [`Storage`](crate::Storage) trait: the
//! primary-region lease view ([`Lease`], spec §8.2; write path step 1,
//! §11.4), and batch coordination state ([`BatchState`], [`BatchStatus`];
//! write path step 4, §11.4; promotion step 2, §13.3).
//!
//! # Scope note: why [`Lease`] is not `coordination::Lease`
//!
//! The already-landed `coordination` crate defines the canonical lease item
//! (`Lease`, `Region`, `HolderId`; spec §8.2/§8.3) over `ReplicatedKv`
//! directly. This crate's ticket (`mtc-f35`) scopes its dependency graph to
//! `cloud-types` + `cloud-backend` + `mtc` only (the Storage facade's
//! crate-layout AC, §9.3-9.4) and does not depend on `coordination`, so
//! [`Lease`] here is the Storage facade's own minimal view -- just enough for
//! the write path's lease check (§11.4 step 1) to type-check against this
//! ticket's `Unimplemented`-bodied `read_lease`. `region` and `holder_id` are
//! plain `String`s rather than re-declared `Region`/`HolderId` newtypes, to
//! avoid a second, incompatible pair of types with the same names.
//! Reconciling this with `coordination::Lease` (reuse, conversion, or a
//! deliberate widening of this crate's dependency boundary) is for whichever
//! sub-ticket wires `read_lease`'s real body.

use std::time::SystemTime;

use mtc::{BatchId, Epoch, Index};

/// The Storage facade's view of the primary-region lease (spec §8.2; write
/// path step 1, §11.4). See the module docs for why this is not
/// `coordination::Lease`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// Region the current holder runs in.
    pub region: String,
    /// Identity of the current holder.
    pub holder_id: String,
    /// Current fencing epoch -- advanced by every takeover (spec §8.3).
    pub epoch: Epoch,
    /// Instant the lease expires; the holder renews before this, challengers
    /// may take over past it plus a safety margin.
    pub expires_at: SystemTime,
}

/// A batch's coordination-state lifecycle (spec §11.1 step 4, §11.2, §13.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchStatus {
    /// Indices allocated and entries in flight; not yet committed (write path
    /// step 4, §11.4).
    Pending,
    /// Committed at the step-8 linearization point (§11.2; write path step 8,
    /// §11.4).
    Committed,
    /// Abandoned during promotion; its indices become permanent `null_entry`
    /// gaps rather than being reused (§11.2; promotion step 2, §13.3).
    Abandoned,
}

/// A batch's persisted coordination-state record (write path step 4, §11.4;
/// promotion step 2, §13.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchState {
    /// The batch's identifier.
    pub batch_id: BatchId,
    /// First allocated index (inclusive).
    pub start: Index,
    /// One past the last allocated index -- the half-open range
    /// `[start, end)`.
    pub end: Index,
    /// Current lifecycle status.
    pub status: BatchStatus,
    /// Fencing epoch the batch's indices were allocated under.
    pub epoch: Epoch,
}
