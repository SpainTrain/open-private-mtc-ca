//! Spec §22.2 guarantee: `CheckpointBuilder::build` is unreachable until
//! `root_hash`, `tree_size`, and `signed_at` are all set. Here `signed_at` is
//! never supplied, so the builder's type has no `build` method and this must
//! not compile.

use mtc::checkpoint::CheckpointBuilder;
use mtc::{HashOutput, LogId, TreeSize};

fn main() {
    let _checkpoint = CheckpointBuilder::new(LogId::new("ca").unwrap())
        .root_hash(HashOutput([0u8; 32]))
        .tree_size(TreeSize(1))
        // .signed_at(..) deliberately omitted — `build` is not available yet.
        .build();
}
