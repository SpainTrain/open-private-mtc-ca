//! Spec section 22.5 guarantee: `LogId` and `BatchId` are distinct
//! compile-time types even though both are `Id<_>` with identical runtime
//! representation.

use mtc::{BatchId, LogId};

fn abandon_batch(_batch: BatchId) {}

fn main() {
    let log = LogId::new("prod-log-1").unwrap();
    abandon_batch(log);
}
