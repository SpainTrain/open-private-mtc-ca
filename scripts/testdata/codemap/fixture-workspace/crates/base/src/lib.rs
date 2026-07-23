//! Fixture crate for scripts/codemap-smoke-test.sh.
//!
//! Exercises: a public module (`alpha`), a private module whose selected
//! items are re-exported instead of the module itself (`beta`), a
//! single-line grouped `pub use`, and a multi-line grouped `pub use` with a
//! renamed (`as`) export — the exact shape (`pub use mod::{\n    A, B,\n};`)
//! that a prior codemap-generator attempt mangled with `sed 's/{.*//'`,
//! silently dropping every symbol after the opening brace.

pub mod alpha;
mod beta;

pub use alpha::{Gadget, Widget, GIZMO_VERSION};

pub use beta::{
    frobnicate, FrobError, Frobnicator,
    Renamed as Aliased,
};
