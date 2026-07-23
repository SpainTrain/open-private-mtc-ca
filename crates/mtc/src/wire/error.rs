//! Structured codec errors for the TLS-presentation wire format.
//!
//! Parsing is the untrusted-input boundary of the library (spec section 19.3:
//! "Untrusted bytes from the network must never panic the service"). Every way
//! a parse can fail is one of these `thiserror` variants (rule
//! `thiserror-for-libs-eyre-for-bins`, spec section 22.6), and every one is a
//! returned `Err` — never a panic.
//!
//! Each variant carries the byte `offset` at which the fault was detected so a
//! caller, a fuzzing regression fixture, or a differential-testing harness
//! (spec section 19.3 layer 4) can localize a malformed input. The enum is
//! `Clone + PartialEq + Eq` so tests can pin an exact expected error (offset
//! plus kind) as a known answer.

use thiserror::Error;

/// An error produced while decoding TLS-presentation-language wire bytes.
///
/// The reporting convention is "expected vs. found at an offset": every
/// variant names where the fault is (`offset`), what the decoder needed, and
/// what the input actually provided. See the module docs for why this is the
/// complete, non-panicking failure set for the untrusted-input path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WireError {
    /// A fixed-width read (integer, fixed-size opaque field, or a length
    /// prefix itself) needed more bytes than remained in the input.
    #[error(
        "unexpected end of input at offset {offset}: \
         needed {needed} more byte(s) but {remaining} remain"
    )]
    UnexpectedEof {
        /// Byte offset (in the top-level input) at which the read was tried.
        offset: usize,
        /// Number of additional bytes the read required.
        needed: usize,
        /// Number of bytes actually remaining at `offset`.
        remaining: usize,
    },

    /// A length prefix announced a body longer than the bytes that follow it.
    ///
    /// This is the "impossible size" rejection of spec section 19.3: the
    /// decoder refuses the claim *before* sizing any allocation to it, so an
    /// attacker cannot force an unbounded allocation with a large length field.
    #[error(
        "length prefix at offset {offset} claims {claimed} byte(s) \
         but only {remaining} remain"
    )]
    LengthOverflow {
        /// Byte offset of the length prefix that made the claim.
        offset: usize,
        /// Body length announced by the prefix.
        claimed: usize,
        /// Bytes remaining after the prefix was read.
        remaining: usize,
    },

    /// Bytes remained after a value — or a length-delimited sub-structure —
    /// was fully parsed.
    ///
    /// Top-level parses ([`crate::wire::TlsParse::tls_parse_exact`]) and
    /// length-prefixed sub-parses both require *exact* consumption; a trailing
    /// suffix is rejected rather than silently ignored (spec section 19.3
    /// single-pass discipline). Silent acceptance of trailing bytes is a
    /// classic parser-differential and signature-substitution vector.
    #[error("{remaining} trailing byte(s) at offset {offset} after value fully parsed")]
    TrailingBytes {
        /// Offset of the first unconsumed byte.
        offset: usize,
        /// Number of unconsumed bytes.
        remaining: usize,
    },

    /// Nested length-prefixed structures exceeded the reader's depth budget.
    ///
    /// Bounds parser recursion so a crafted, deeply nested input cannot exhaust
    /// the stack (spec section 19.3 asserted property: "No infinite loops
    /// (parser has bounded depth)").
    #[error("maximum nesting depth {limit} exceeded at offset {offset}")]
    DepthLimitExceeded {
        /// Offset at which the over-deep nesting level was entered.
        offset: usize,
        /// The configured maximum depth ([`crate::wire::TlsReader::DEFAULT_MAX_DEPTH`]).
        limit: usize,
    },

    /// A length-prefixed vector contained an element that consumed zero bytes.
    ///
    /// Parsing a vector loops until its bounded body is exhausted; an element
    /// codec that consumes nothing would loop forever on a non-empty body.
    /// Rejecting it upholds the spec section 19.3 property "No infinite loops"
    /// and keeps parse time bounded by input length, not content.
    #[error("zero-width vector element at offset {offset} would not terminate the parse loop")]
    ZeroWidthElement {
        /// Offset of the element that failed to advance the cursor.
        offset: usize,
    },
}
