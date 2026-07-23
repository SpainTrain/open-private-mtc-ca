//! Bounded, single-pass reader for TLS-presentation-language wire bytes.
//!
//! `TlsReader` is the security boundary of the codec (spec section 19.3):
//! untrusted network bytes enter the library here and this type upholds the
//! properties the spec asserts across all fuzzing layers:
//!
//! - **No panic on any input.** Every read is guarded; out-of-range reads
//!   return [`WireError`], never index out of bounds.
//! - **No unbounded allocation.** Every length prefix is validated against the
//!   bytes actually remaining *before* it is used to bound a slice or size a
//!   `Vec` ([`WireError::LengthOverflow`]).
//! - **No infinite loops.** Vector parsing rejects zero-width elements
//!   ([`WireError::ZeroWidthElement`]) and nesting is bounded by a depth budget
//!   ([`WireError::DepthLimitExceeded`]).
//! - **Time bounded by input length, not content.** The cursor only moves
//!   forward (single pass); each input byte is examined a constant number of
//!   times.
//! - **Trailing bytes rejected.** [`TlsReader::finish`] fails if any byte is
//!   left unconsumed, so a value with an unexpected suffix is not silently
//!   accepted.
//!
//! Arithmetic on offsets and lengths is checked (`checked_add`) or expressed as
//! a subtraction whose non-negativity is guaranteed by the `pos <= buf.len()`
//! invariant, so no length computation can wrap (crypto-review checklist:
//! bounded parsing).

use super::error::WireError;
use super::U24;

/// A cursor over an immutable byte slice that decodes TLS-presentation-language
/// primitives without ever reading out of bounds.
///
/// # Invariants
///
/// - `pos <= buf.len()` at all times. Every method that advances the cursor
///   first proves the advance stays in bounds, so [`Self::remaining`] is a
///   non-wrapping subtraction.
/// - `base_offset` is the absolute offset of `buf[0]` within the original
///   top-level input. It is `0` for a reader made with [`Self::new`] and is set
///   to the sub-structure's start when a length-prefixed body is parsed in a
///   bounded sub-reader, so error offsets stay absolute even inside nested
///   structures.
#[derive(Debug)]
pub struct TlsReader<'a> {
    buf: &'a [u8],
    pos: usize,
    base_offset: usize,
    depth: usize,
    max_depth: usize,
}

impl<'a> TlsReader<'a> {
    /// Default maximum nesting depth for length-prefixed sub-structures.
    ///
    /// Chosen well above the nesting of any spec structure yet small enough
    /// that recursion cannot approach the stack limit; callers with unusual
    /// needs override it via [`Self::with_max_depth`].
    pub const DEFAULT_MAX_DEPTH: usize = 32;

    /// Creates a reader positioned at the start of `buf`.
    #[must_use]
    pub const fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            base_offset: 0,
            depth: 0,
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }

    /// Creates a reader with an explicit maximum nesting depth.
    #[must_use]
    pub const fn with_max_depth(buf: &'a [u8], max_depth: usize) -> Self {
        Self {
            buf,
            pos: 0,
            base_offset: 0,
            depth: 0,
            max_depth,
        }
    }

    /// Number of bytes consumed from this reader's slice so far.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Number of bytes still available to read.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        // Non-wrapping: the `pos <= buf.len()` invariant holds after every read.
        self.buf.len() - self.pos
    }

    /// Whether the reader has consumed its entire slice.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Absolute offset of the cursor within the top-level input.
    const fn offset(&self) -> usize {
        // `base_offset + pos` cannot wrap: both are bounded by the length of the
        // original input, which is a valid `usize`.
        self.base_offset + self.pos
    }

    /// Reads exactly `n` bytes, advancing the cursor.
    ///
    /// The length is checked against the buffer with [`usize::checked_add`]
    /// before any slice is formed, so the returned slice is always in bounds
    /// and no arithmetic can wrap.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if fewer than `n` bytes remain. The cursor
    /// is left unchanged, so the reported offset points at the failed read.
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let start = self.pos;
        let fits = match start.checked_add(n) {
            Some(end) => end <= self.buf.len(),
            None => false,
        };
        if !fits {
            return Err(WireError::UnexpectedEof {
                offset: self.offset(),
                needed: n,
                remaining: self.remaining(),
            });
        }
        let end = start + n; // proven `<= buf.len()` above
        match self.buf.get(start..end) {
            Some(slice) => {
                self.pos = end;
                Ok(slice)
            }
            // Unreachable given the bounds check above; handled without a panic
            // (rule no-unwrap-in-prod) so a future refactor cannot turn a logic
            // slip into an out-of-bounds panic.
            None => Err(WireError::UnexpectedEof {
                offset: self.offset(),
                needed: n,
                remaining: self.remaining(),
            }),
        }
    }

    /// Reads a fixed-size array of `N` bytes (TLS `opaque field[N]`).
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if fewer than `N` bytes remain.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let slice = self.read_bytes(N)?;
        let mut array = [0u8; N];
        // `slice.len() == N` by construction of `read_bytes`, so this cannot
        // panic on a length mismatch.
        array.copy_from_slice(slice);
        Ok(array)
    }

    /// Reads a big-endian `uint8`.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if no bytes remain.
    pub fn read_u8(&mut self) -> Result<u8, WireError> {
        let [b] = self.read_array::<1>()?;
        Ok(b)
    }

    /// Reads a big-endian `uint16`.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if fewer than 2 bytes remain.
    pub fn read_u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.read_array::<2>()?))
    }

    /// Reads a big-endian `uint24`.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if fewer than 3 bytes remain.
    pub fn read_u24(&mut self) -> Result<U24, WireError> {
        let [b0, b1, b2] = self.read_array::<3>()?;
        let value = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        // Value occupies 24 bits by construction, so it is always in range.
        Ok(U24::from_low_24(value))
    }

    /// Reads a big-endian `uint32`.
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if fewer than 4 bytes remain.
    pub fn read_u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.read_array::<4>()?))
    }

    /// Validates a body length against the remaining input and returns the body
    /// slice, advancing the cursor.
    ///
    /// The length is compared with [`Self::remaining`] *before* the slice is
    /// formed, so a prefix claiming an impossible size is rejected as
    /// [`WireError::LengthOverflow`] with no allocation sized to the claim.
    fn take_body(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        if len > self.remaining() {
            return Err(WireError::LengthOverflow {
                offset: self.offset(),
                claimed: len,
                remaining: self.remaining(),
            });
        }
        self.read_bytes(len)
    }

    /// Reads a `u8`-length-prefixed opaque byte string (`opaque v<0..2^8-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if the length prefix is truncated;
    /// [`WireError::LengthOverflow`] if it claims more bytes than remain.
    pub fn read_opaque_u8(&mut self) -> Result<&'a [u8], WireError> {
        let len = usize::from(self.read_u8()?);
        self.take_body(len)
    }

    /// Reads a `u16`-length-prefixed opaque byte string (`opaque v<0..2^16-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if the length prefix is truncated;
    /// [`WireError::LengthOverflow`] if it claims more bytes than remain.
    pub fn read_opaque_u16(&mut self) -> Result<&'a [u8], WireError> {
        let len = usize::from(self.read_u16()?);
        self.take_body(len)
    }

    /// Reads a `u24`-length-prefixed opaque byte string (`opaque v<0..2^24-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError::UnexpectedEof`] if the length prefix is truncated;
    /// [`WireError::LengthOverflow`] if it claims more bytes than remain.
    pub fn read_opaque_u24(&mut self) -> Result<&'a [u8], WireError> {
        // `usize::try_from` only fails on a <32-bit `usize`; there a 24-bit
        // length that does not fit is necessarily larger than any possible
        // buffer, so mapping it to `usize::MAX` yields the correct
        // `LengthOverflow` rejection rather than a panic.
        let len = usize::try_from(self.read_u24()?.get()).unwrap_or(usize::MAX);
        self.take_body(len)
    }

    /// Runs `body` against a sub-reader bounded to the next `len` bytes,
    /// requiring it to consume all of them, and increments the depth budget.
    ///
    /// This is the single choke point for length-delimited sub-structures. It
    /// enforces three spec section 19.3 properties at once: bounded allocation
    /// (the sub-reader cannot see beyond `len`, already validated against
    /// `remaining`), bounded depth (each level checks and increments `depth`),
    /// and exact consumption (leftover bytes inside the sub-structure become
    /// [`WireError::TrailingBytes`]).
    fn read_scoped<T>(
        &mut self,
        len: usize,
        body: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.depth >= self.max_depth {
            return Err(WireError::DepthLimitExceeded {
                offset: self.offset(),
                limit: self.max_depth,
            });
        }
        let body_abs = self.offset();
        let body_bytes = self.take_body(len)?;
        let mut sub = Self {
            buf: body_bytes,
            pos: 0,
            base_offset: body_abs,
            depth: self.depth + 1,
            max_depth: self.max_depth,
        };
        let value = body(&mut sub)?;
        if !sub.is_empty() {
            return Err(WireError::TrailingBytes {
                offset: sub.offset(),
                remaining: sub.remaining(),
            });
        }
        Ok(value)
    }

    /// Parses `T` items from a length-prefixed body until it is exhausted.
    ///
    /// Rejects a zero-width element, which would otherwise spin forever on a
    /// non-empty body.
    fn read_items<T: super::TlsParse>(&mut self, len: usize) -> Result<Vec<T>, WireError> {
        self.read_scoped(len, |sub| {
            let mut items = Vec::new();
            while !sub.is_empty() {
                let before = sub.position();
                items.push(T::tls_parse(sub)?);
                if sub.position() == before {
                    return Err(WireError::ZeroWidthElement {
                        offset: sub.offset(),
                    });
                }
            }
            Ok(items)
        })
    }

    /// Reads a `u8`-length-prefixed vector of `T` (`T v<0..2^8-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError`] if the prefix is truncated, claims an impossible size, a
    /// contained item fails to parse, or an item consumes zero bytes.
    pub fn read_vector_u8<T: super::TlsParse>(&mut self) -> Result<Vec<T>, WireError> {
        let len = usize::from(self.read_u8()?);
        self.read_items(len)
    }

    /// Reads a `u16`-length-prefixed vector of `T` (`T v<0..2^16-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError`] if the prefix is truncated, claims an impossible size, a
    /// contained item fails to parse, or an item consumes zero bytes.
    pub fn read_vector_u16<T: super::TlsParse>(&mut self) -> Result<Vec<T>, WireError> {
        let len = usize::from(self.read_u16()?);
        self.read_items(len)
    }

    /// Reads a `u24`-length-prefixed vector of `T` (`T v<0..2^24-1>`).
    ///
    /// # Errors
    ///
    /// [`WireError`] if the prefix is truncated, claims an impossible size, a
    /// contained item fails to parse, or an item consumes zero bytes.
    pub fn read_vector_u24<T: super::TlsParse>(&mut self) -> Result<Vec<T>, WireError> {
        let len = usize::try_from(self.read_u24()?.get()).unwrap_or(usize::MAX);
        self.read_items(len)
    }

    /// Asserts the entire input has been consumed.
    ///
    /// Called by [`super::TlsParse::tls_parse_exact`] so that a top-level parse
    /// rejects any trailing suffix (spec section 19.3).
    ///
    /// # Errors
    ///
    /// [`WireError::TrailingBytes`] if any bytes remain unconsumed.
    pub const fn finish(&self) -> Result<(), WireError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes {
                offset: self.offset(),
                remaining: self.remaining(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TlsReader, WireError};
    use crate::wire::{TlsParse, U24};

    #[test]
    fn reads_primitives_big_endian_and_tracks_position() {
        let mut r = TlsReader::new(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A]);
        assert_eq!(r.read_u8(), Ok(0x01));
        assert_eq!(r.position(), 1);
        assert_eq!(r.read_u16(), Ok(0x0203));
        assert_eq!(r.read_u24().map(U24::get), Ok(0x0004_0506));
        assert_eq!(r.read_u32(), Ok(0x0708_090A));
        assert!(r.is_empty());
        assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn truncated_integer_reports_eof_with_offset() {
        let mut r = TlsReader::new(&[0xAA]);
        assert_eq!(
            r.read_u32(),
            Err(WireError::UnexpectedEof {
                offset: 0,
                needed: 4,
                remaining: 1,
            }),
        );
        // Cursor unchanged after a failed read, so the offset stays meaningful.
        assert_eq!(r.position(), 0);
    }

    #[test]
    fn opaque_vector_rejects_impossible_length_without_allocating() {
        // A u16 length prefix of 0xFFFF with an empty body: the claim is
        // refused before any 65535-byte allocation is sized.
        let mut r = TlsReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            r.read_opaque_u16(),
            Err(WireError::LengthOverflow {
                offset: 2,
                claimed: 0xFFFF,
                remaining: 0,
            }),
        );
    }

    #[test]
    fn opaque_vector_reads_exact_body() {
        let mut r = TlsReader::new(&[0x03, 0xDE, 0xAD, 0xBE, 0x99]);
        assert_eq!(r.read_opaque_u8(), Ok(&[0xDE, 0xAD, 0xBE][..]));
        // The trailing 0x99 is outside the opaque field and still available.
        assert_eq!(r.remaining(), 1);
        assert_eq!(r.read_u8(), Ok(0x99));
    }

    #[test]
    fn typed_vector_parses_all_elements() {
        // u16-prefixed vector of three u16 items: len = 6 bytes.
        let bytes = [0x00, 0x06, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03];
        let mut r = TlsReader::new(&bytes);
        let items: Vec<u16> = r.read_vector_u16().unwrap();
        assert_eq!(items, vec![0x0001, 0x0002, 0x0003]);
        assert!(r.is_empty());
    }

    #[test]
    fn typed_vector_rejects_body_that_misaligns_with_elements() {
        // Body length 3 but u16 elements are 2 bytes wide: the second element
        // reads past the bounded sub-slice -> EOF inside the scope.
        let bytes = [0x00, 0x03, 0x00, 0x01, 0x02];
        let mut r = TlsReader::new(&bytes);
        let parsed: Result<Vec<u16>, _> = r.read_vector_u16();
        assert!(
            matches!(parsed, Err(WireError::UnexpectedEof { .. })),
            "{parsed:?}"
        );
    }

    // A recursive type whose only job is to nest length-prefixed scopes so the
    // depth guard can be exercised: each `Nest` is a u8-prefixed vector holding
    // exactly one child `Nest` (or nothing, at the leaf).
    #[derive(Debug, PartialEq)]
    struct Nest(Vec<Self>);

    impl TlsParse for Nest {
        fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
            Ok(Self(reader.read_vector_u8()?))
        }
    }

    /// Builds `depth` nested single-child vectors: `[len,[len,[...]]]`.
    fn nested_bytes(depth: usize) -> Vec<u8> {
        let mut buf = vec![0x00]; // innermost leaf: empty u8-prefixed vector
        for _ in 0..depth {
            let mut outer = Vec::with_capacity(buf.len() + 1);
            let len = u8::try_from(buf.len()).expect("test nesting stays under 256 bytes");
            outer.push(len);
            outer.extend_from_slice(&buf);
            buf = outer;
        }
        buf
    }

    #[test]
    fn nesting_within_budget_parses() {
        // DEFAULT_MAX_DEPTH scopes are allowed; build a few levels under it.
        let bytes = nested_bytes(4);
        assert!(Nest::tls_parse_exact(&bytes).is_ok());
    }

    #[test]
    fn nesting_beyond_budget_is_rejected_not_stack_overflow() {
        let bytes = nested_bytes(8);
        let mut reader = TlsReader::with_max_depth(&bytes, 2);
        let parsed = Nest::tls_parse(&mut reader);
        assert!(
            matches!(parsed, Err(WireError::DepthLimitExceeded { limit: 2, .. })),
            "{parsed:?}",
        );
    }

    // A codec that deliberately consumes zero bytes, to prove the vector loop
    // cannot be driven into a non-terminating spin.
    #[derive(Debug, PartialEq)]
    struct ZeroWidth;

    impl TlsParse for ZeroWidth {
        fn tls_parse(_reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
            Ok(Self)
        }
    }

    #[test]
    fn zero_width_vector_element_is_rejected() {
        // A u8-prefixed body of length 1; the ZeroWidth codec never advances,
        // so the loop would spin forever without the guard.
        let bytes = [0x01, 0x00];
        let mut r = TlsReader::new(&bytes);
        let parsed: Result<Vec<ZeroWidth>, _> = r.read_vector_u8();
        assert!(
            matches!(parsed, Err(WireError::ZeroWidthElement { .. })),
            "{parsed:?}"
        );
    }

    #[test]
    fn nested_error_offsets_are_absolute() {
        // Outer u8-vector body of length 3 starting at offset 1; inside it a
        // u16 read at relative 0 (absolute 1) then a truncated u16 at absolute
        // 3 should report the absolute offset.
        let bytes = [0x03, 0x00, 0x01, 0xFF];
        let mut r = TlsReader::new(&bytes);
        // Body is 3 bytes: one whole u16 (0x0001) leaves 1 byte, so a second
        // u16 fails with EOF at absolute offset 3.
        let parsed: Result<Vec<u16>, _> = r.read_vector_u8();
        assert_eq!(
            parsed,
            Err(WireError::UnexpectedEof {
                offset: 3,
                needed: 2,
                remaining: 1,
            }),
        );
    }
}
