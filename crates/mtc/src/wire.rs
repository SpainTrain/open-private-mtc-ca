//! TLS-presentation-language serialization framework with bounded parsing.
//!
//! Spec structures use the TLS presentation language (spec section 4 table row
//! "Serialization"; the language itself is RFC 8446 section 3). Hand-written
//! wire-format code is a named risk area (spec section 19.3), so this module
//! provides a small, auditable framework that every spec-type codec ticket
//! (Checkpoint, tiles, entries, proofs) builds on, rather than each re-deriving
//! primitive encode/decode:
//!
//! - [`TlsSerialize`] / [`TlsParse`] — the two codec traits. Serialization is
//!   generic over the sink (`<W: Write>`) for static dispatch on the hot path
//!   (spec section 22.7). Parsing goes through the bounded [`TlsReader`].
//! - Primitives: [`U24`] plus `impl`s for `u8`, `u16`, `u32`, and fixed-size
//!   opaque `[u8; N]`.
//! - Length-prefixed opaque strings and vectors, via [`TlsReader`] methods and
//!   the `write_*` functions.
//! - [`assert_roundtrip!`](crate::assert_roundtrip) — the shared round-trip
//!   (and optional known-answer) test helper reused by every later codec
//!   ticket (spec section 19.2).
//!
//! # Bounded-parsing guarantees (spec section 19.3)
//!
//! [`TlsReader`] is the untrusted-input boundary. It never panics, validates
//! every length against the bytes actually remaining before sizing any
//! allocation, bounds nesting depth and rejects zero-width vector elements (no
//! infinite loops), is single-pass (parse time bounded by input length, not
//! content), and rejects trailing bytes. See the [`reader`] module docs for the
//! per-property detail.
//!
//! # Scope
//!
//! This is the framework only. Per-type codecs for `Checkpoint`, log entries,
//! tiles, and proofs land in their own tickets. Known-answer vectors here pin
//! the *primitive* encodings, which are fully determined by the presentation
//! language (RFC 8446 section 3). Byte-exact vectors for composite spec types
//! must be pinned in those tickets from the draft's own test vectors once the
//! draft provides them; this ticket does not invent them.

mod error;
mod reader;
mod writer;

use std::io::{self, Write};

pub use error::WireError;
pub use reader::TlsReader;
pub use writer::{
    write_bytes, write_opaque_u16, write_opaque_u24, write_opaque_u8, write_u16, write_u24,
    write_u32, write_u8, write_vector_u16, write_vector_u24, write_vector_u8,
};

/// A 24-bit unsigned integer (`uint24` in the TLS presentation language).
///
/// The presentation language has a native `uint24`, but Rust does not, so this
/// newtype carries a `u32` constrained to the range `0..=2^24 - 1`. Modelling
/// it as a distinct type (rather than passing a bare `u32`) keeps an
/// out-of-range value from ever reaching [`write_u24`]: the invariant is
/// established once, at construction.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct U24(u32);

impl U24 {
    /// The largest representable `uint24` value (`2^24 - 1`).
    pub const MAX: Self = Self(0x00FF_FFFF);

    /// Creates a `U24` from a `u32`, returning `None` if it exceeds [`Self::MAX`].
    ///
    /// Modelled on [`core::num::NonZeroU32::new`]: a checked constructor that
    /// makes the range invariant explicit at the call site.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        if value <= Self::MAX.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns the value as a `u32` (always `<= 2^24 - 1`).
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Constructs a `U24` from a value known to occupy at most 24 bits.
    ///
    /// The argument is masked to 24 bits so the range invariant holds even if a
    /// caller passes a wider value; the sole in-crate caller ([`TlsReader::read_u24`])
    /// already supplies a 24-bit value assembled from three bytes.
    pub(crate) const fn from_low_24(value: u32) -> Self {
        Self(value & 0x00FF_FFFF)
    }

    /// Converts a `usize` length to a `U24`, returning `None` if it does not fit.
    pub(crate) fn try_from_usize(len: usize) -> Option<Self> {
        u32::try_from(len).ok().and_then(Self::new)
    }
}

impl From<u8> for U24 {
    fn from(value: u8) -> Self {
        Self(u32::from(value))
    }
}

impl From<u16> for U24 {
    fn from(value: u16) -> Self {
        Self(u32::from(value))
    }
}

/// Serializes a value into the TLS presentation language.
///
/// The primitive writers this composes are generic over the sink (spec section
/// 22.7 hot-path row), so per-byte serialization is statically dispatched.
/// Serialization emits the library's own trusted, well-typed values; the only
/// runtime failure is the sink's I/O, plus the encoding invariant that a
/// variable-length body must fit its length prefix (surfaced as
/// [`std::io::ErrorKind::InvalidData`]).
pub trait TlsSerialize {
    /// Writes `self` to `writer` in wire form.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from `writer`, and
    /// [`std::io::ErrorKind::InvalidData`] if a contained variable-length field
    /// is too long to encode in its length prefix.
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()>;

    /// Serializes `self` into a freshly allocated `Vec<u8>`.
    ///
    /// Convenience for the common case of encoding to memory (and the form used
    /// by [`assert_roundtrip!`](crate::assert_roundtrip)).
    #[must_use]
    fn tls_serialize_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // Writing into a `Vec<u8>` is infallible — its `io::Write` impl never
        // returns `Err` and cannot exceed a length prefix on its own — so no
        // error can arise here. We discard the impossible `Result` without
        // `unwrap`/`expect` (rule no-unwrap-in-prod); a caller that needs a
        // fallible sink calls `tls_serialize` directly.
        let _ = self.tls_serialize(&mut buf);
        buf
    }
}

/// Parses a value from bounded TLS-presentation-language bytes.
///
/// Implementations read through a [`TlsReader`], which guarantees the parse
/// never panics and is bounded in time, allocation, and recursion depth (spec
/// section 19.3).
pub trait TlsParse: Sized {
    /// Parses one value from `reader`, consuming exactly the bytes it needs and
    /// leaving the cursor positioned after them.
    ///
    /// This is the composable form: an aggregate type parses its fields by
    /// calling `tls_parse` on each in turn.
    ///
    /// # Errors
    ///
    /// [`WireError`] on any malformed input. Never panics.
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError>;

    /// Parses a value from a complete byte slice, requiring every byte to be
    /// consumed.
    ///
    /// This is the top-level entry point (the spec's `parse_tls_presentation`):
    /// a trailing suffix is rejected as [`WireError::TrailingBytes`] rather than
    /// silently ignored (spec section 19.3).
    ///
    /// # Errors
    ///
    /// [`WireError`] on malformed input or on trailing bytes.
    fn tls_parse_exact(bytes: &[u8]) -> Result<Self, WireError> {
        let mut reader = TlsReader::new(bytes);
        let value = Self::tls_parse(&mut reader)?;
        reader.finish()?;
        Ok(value)
    }
}

impl TlsSerialize for u8 {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u8(writer, *self)
    }
}

impl TlsParse for u8 {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        reader.read_u8()
    }
}

impl TlsSerialize for u16 {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u16(writer, *self)
    }
}

impl TlsParse for u16 {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        reader.read_u16()
    }
}

impl TlsSerialize for u32 {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u32(writer, *self)
    }
}

impl TlsParse for u32 {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        reader.read_u32()
    }
}

impl TlsSerialize for U24 {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u24(writer, *self)
    }
}

impl TlsParse for U24 {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        reader.read_u24()
    }
}

impl<const N: usize> TlsSerialize for [u8; N] {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_bytes(writer, self)
    }
}

impl<const N: usize> TlsParse for [u8; N] {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        reader.read_array::<N>()
    }
}

/// Serializes `value`, parses the bytes back, and returns both the wire bytes
/// and the re-parsed value.
///
/// This is the engine behind [`assert_roundtrip!`](crate::assert_roundtrip):
/// taking `value` by reference pins the codec type from the argument, so it
/// works uniformly for concrete and generic (`[u8; N]`) implementors where a
/// bare trait-qualified call could not infer `Self`. Tickets that want the
/// round-tripped pair without the assertion can call it directly.
///
/// # Errors
///
/// [`WireError`] if the freshly serialized bytes fail to parse — which, for a
/// correct codec, indicates a serialize/parse asymmetry bug.
pub fn roundtrip<T: TlsSerialize + TlsParse>(value: &T) -> Result<(Vec<u8>, T), WireError> {
    let bytes = value.tls_serialize_to_vec();
    let parsed = T::tls_parse_exact(&bytes)?;
    Ok((bytes, parsed))
}

/// Asserts the round-trip identity `parse(serialize(x)) == x` and returns the
/// serialized bytes.
///
/// This is the shared helper of spec section 19.2 ("Serialization round-trips")
/// that every later spec-type codec ticket reuses. The value must implement
/// [`TlsSerialize`] + [`TlsParse`] + `PartialEq` + `Debug`.
///
/// A second argument pins the exact wire encoding as a known-answer vector —
/// use it to lock a byte format against a draft test vector:
///
/// ```
/// use mtc::assert_roundtrip;
///
/// // One-argument form: round-trip only, returns the bytes.
/// let bytes = assert_roundtrip!(0x0102_u16);
/// assert_eq!(bytes, vec![0x01, 0x02]);
///
/// // Two-argument form: also pins the encoding (big-endian, RFC 8446 §3).
/// assert_roundtrip!(0x0102_u16, [0x01, 0x02]);
/// ```
///
/// The macro uses `assert_eq!`/`panic!` (not `unwrap`/`expect`), so it is inert
/// with respect to the production `unwrap`-ban lint; it is nonetheless a test
/// helper.
#[macro_export]
macro_rules! assert_roundtrip {
    ($value:expr $(,)?) => {{
        let __value = $value;
        match $crate::wire::roundtrip(&__value) {
            ::core::result::Result::Ok((__bytes, __parsed)) => {
                ::core::assert_eq!(
                    __value,
                    __parsed,
                    "round-trip mismatch: parse(serialize(x)) != x",
                );
                __bytes
            }
            ::core::result::Result::Err(__err) => {
                ::core::panic!("round-trip parse failed on serialized bytes: {:?}", __err);
            }
        }
    }};
    ($value:expr, $expected:expr $(,)?) => {{
        let __bytes = $crate::assert_roundtrip!($value);
        ::core::assert_eq!(
            &__bytes[..],
            &$expected[..],
            "serialized bytes do not match the pinned known-answer vector",
        );
        __bytes
    }};
}

#[cfg(test)]
mod tests {
    use super::{TlsParse, TlsSerialize, WireError, U24};

    #[test]
    fn u24_new_enforces_range() {
        assert_eq!(U24::new(0).map(U24::get), Some(0));
        assert_eq!(U24::new(0x00FF_FFFF).map(U24::get), Some(0x00FF_FFFF));
        assert_eq!(U24::new(0x0100_0000), None);
        assert_eq!(U24::new(u32::MAX), None);
        assert_eq!(U24::MAX.get(), 0x00FF_FFFF);
    }

    #[test]
    fn u24_from_small_ints_is_infallible() {
        assert_eq!(U24::from(0xABu8).get(), 0x0000_00AB);
        assert_eq!(U24::from(0xBEEFu16).get(), 0x0000_BEEF);
    }

    #[test]
    fn primitive_known_answer_vectors() {
        // Encodings are fixed by the presentation language (RFC 8446 §3):
        // big-endian, no padding.
        assert_eq!(0x12u8.tls_serialize_to_vec(), vec![0x12]);
        assert_eq!(0x0102u16.tls_serialize_to_vec(), vec![0x01, 0x02]);
        assert_eq!(
            U24::new(0x0001_0203).unwrap().tls_serialize_to_vec(),
            vec![0x01, 0x02, 0x03],
        );
        assert_eq!(
            0x0102_0304u32.tls_serialize_to_vec(),
            vec![0x01, 0x02, 0x03, 0x04],
        );
        // Fixed-size opaque has no length prefix.
        assert_eq!(
            [0xAAu8, 0xBB, 0xCC].tls_serialize_to_vec(),
            vec![0xAA, 0xBB, 0xCC]
        );
    }

    #[test]
    fn primitive_round_trips() {
        for v in [0u8, 1, 0x7F, 0x80, 0xFF] {
            assert_eq!(u8::tls_parse_exact(&v.tls_serialize_to_vec()), Ok(v));
        }
        for v in [0u16, 1, 0x00FF, 0x0100, 0xFFFF] {
            assert_eq!(u16::tls_parse_exact(&v.tls_serialize_to_vec()), Ok(v));
        }
        for v in [0u32, 1, 0x00FF_FFFF, 0x0100_0000, u32::MAX] {
            assert_eq!(u32::tls_parse_exact(&v.tls_serialize_to_vec()), Ok(v));
        }
        for raw in [0u32, 1, 0x00FF_FFFF, 0x0012_3456] {
            let v = U24::new(raw).unwrap();
            assert_eq!(U24::tls_parse_exact(&v.tls_serialize_to_vec()), Ok(v));
        }
    }

    #[test]
    fn fixed_opaque_round_trips_and_rejects_short_input() {
        let value = [0x01u8, 0x02, 0x03, 0x04];
        let bytes = value.tls_serialize_to_vec();
        assert_eq!(<[u8; 4]>::tls_parse_exact(&bytes), Ok(value));
        // One byte short: EOF, not a panic.
        assert!(matches!(
            <[u8; 4]>::tls_parse_exact(&[0x01, 0x02, 0x03]),
            Err(WireError::UnexpectedEof { .. }),
        ));
    }

    #[test]
    fn tls_parse_exact_rejects_trailing_bytes() {
        // A valid u16 (2 bytes) followed by an extra byte must be rejected.
        assert_eq!(
            u16::tls_parse_exact(&[0x01, 0x02, 0x03]),
            Err(WireError::TrailingBytes {
                offset: 2,
                remaining: 1,
            }),
        );
    }

    #[test]
    fn assert_roundtrip_macro_one_and_two_arg_forms() {
        let bytes = crate::assert_roundtrip!(0x0102_u16);
        assert_eq!(bytes, vec![0x01, 0x02]);
        crate::assert_roundtrip!(0xAB_u8, [0xAB]);
        crate::assert_roundtrip!(U24::new(0x01_02_03).unwrap(), [0x01, 0x02, 0x03]);
    }
}
