//! Fuzz target: `PruningCheckpoint` wire parsing must be total (spec §19.3;
//! ticket `prune-checkpoint-format`).
//!
//! Feeds arbitrary bytes to `PruningCheckpoint::tls_parse_exact` — the exact
//! entry point a pruning-checkpoint consumer would use on untrusted input
//! (read from S3, replicated state, or an admin API request body). Asserts
//! nothing panics; a structured `Err` is the expected outcome for malformed
//! input, including the two range invariants spec §15.2 requires
//! (`pruned_start <= pruned_end <= tree_size`) and the hand-enforced
//! non-empty `signing_key_id` floor (crypto F3).

#![no_main]

use libfuzzer_sys::fuzz_target;
use mtc::{PruningCheckpoint, TlsParse};

fuzz_target!(|data: &[u8]| {
    let _ = PruningCheckpoint::tls_parse_exact(data);
});
