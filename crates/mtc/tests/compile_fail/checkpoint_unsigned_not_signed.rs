//! Spec §22.4 guarantee: an unsigned checkpoint cannot be used where a signed
//! one is required. `Checkpoint<Unsigned>` and `Checkpoint<Signed>` are
//! distinct types, so passing the former where the latter is expected must not
//! compile.

use mtc::checkpoint::{Checkpoint, CheckpointBuilder, Signed, Unsigned};
use mtc::{HashOutput, LogId, SignedAt, TreeSize};

fn requires_signed(_checkpoint: Checkpoint<Signed>) {}

fn main() {
    let unsigned: Checkpoint<Unsigned> = CheckpointBuilder::new(LogId::new("ca").unwrap())
        .root_hash(HashOutput([0u8; 32]))
        .tree_size(TreeSize(1))
        .signed_at(SignedAt(0))
        .build();
    // An unsigned checkpoint is not a signed checkpoint (spec §22.4).
    requires_signed(unsigned);
}
