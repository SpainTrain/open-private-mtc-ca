//! Spec §22.2 typestate guarantee (ticket `mtclib-log-entries` AC): a
//! `TbsCertificateLogEntry` cannot be built until both required fields —
//! `subject_type` and `subject_info_hash` — are set. `.build()` exists only on
//! the fully-populated builder state, so calling it on a fresh (incomplete)
//! builder is a compile error, not a runtime one.

use mtc::TbsCertificateLogEntry;

fn main() {
    // Neither required field set: no `build` method exists in this state.
    let _entry = TbsCertificateLogEntry::builder().build();
}
