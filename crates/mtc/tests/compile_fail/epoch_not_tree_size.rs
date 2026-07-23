//! Spec section 22.1 guarantee: the compiler refuses to pass an `Epoch`
//! where a `TreeSize` is expected. Newtypes, not aliases (rule use-newtypes).

use mtc::{Epoch, TreeSize};

fn commit_checkpoint(_tree_size: TreeSize) {}

fn main() {
    let epoch = Epoch(7);
    commit_checkpoint(epoch);
}
