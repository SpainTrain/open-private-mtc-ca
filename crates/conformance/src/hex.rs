//! Minimal hex codec for the vector schema's `*_hex` fields.
//!
//! A dedicated `hex` crate is not otherwise used anywhere in this workspace
//! (checked at ticket time), so this is a small hand-rolled codec rather than
//! a new external dependency for two one-screen functions.

use std::fmt::Write as _;

use thiserror::Error;

/// Encodes `bytes` as lowercase hex with no separators or `0x` prefix — the
/// convention every `*_hex` vector field uses.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // `write!` to the String directly (no intermediate `format!`
        // allocation per byte); infallible for a `String` sink.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Decodes a hex string (case-insensitive, no separators or `0x` prefix) to
/// bytes.
///
/// # Errors
///
/// [`HexError::OddLength`] if `s` has an odd number of characters;
/// [`HexError::InvalidDigit`] if any character is not `[0-9a-fA-F]`.
pub fn decode(s: &str) -> Result<Vec<u8>, HexError> {
    if !s.len().is_multiple_of(2) {
        return Err(HexError::OddLength { len: s.len() });
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_digit(bytes[i], i)?;
        let lo = hex_digit(bytes[i + 1], i + 1)?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

/// Decodes one ASCII hex digit at `offset` (for error reporting) to its
/// 4-bit value.
const fn hex_digit(byte: u8, offset: usize) -> Result<u8, HexError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(HexError::InvalidDigit {
            offset,
            found: byte as char,
        }),
    }
}

/// A hex string could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HexError {
    /// The string's length was not a multiple of 2.
    #[error("hex string has odd length {len}")]
    OddLength {
        /// The offending length.
        len: usize,
    },
    /// A character was not an ASCII hex digit.
    #[error("invalid hex digit {found:?} at offset {offset}")]
    InvalidDigit {
        /// The character's byte offset in the input string.
        offset: usize,
        /// The offending character.
        found: char,
    },
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, HexError};

    #[test]
    fn round_trips() {
        let bytes = [0x00u8, 0x01, 0xAB, 0xFF, 0x7f];
        let hex = encode(&bytes);
        assert_eq!(hex, "0001abff7f");
        assert_eq!(decode(&hex).unwrap(), bytes);
    }

    #[test]
    fn decode_is_case_insensitive() {
        assert_eq!(decode("AbCd").unwrap(), decode("abcd").unwrap());
    }

    #[test]
    fn empty_string_decodes_to_empty_bytes() {
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
        assert_eq!(encode(&[]), "");
    }

    #[test]
    fn rejects_odd_length() {
        assert_eq!(decode("abc"), Err(HexError::OddLength { len: 3 }));
    }

    #[test]
    fn rejects_invalid_digit() {
        assert_eq!(
            decode("zz"),
            Err(HexError::InvalidDigit {
                offset: 0,
                found: 'z',
            }),
        );
        assert_eq!(
            decode("0g"),
            Err(HexError::InvalidDigit {
                offset: 1,
                found: 'g',
            }),
        );
    }
}
