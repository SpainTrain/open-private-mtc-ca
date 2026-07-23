//! Primitive TLS-presentation-language writers.
//!
//! The write path serializes the library's own trusted, well-typed values.
//! Each primitive writer is generic over the sink (`<W: Write>`, spec section
//! 22.7: "Serialization primitives | generic `<W: Write>` | Hot inner loops;
//! static dispatch matters"), so per-byte serialization is monomorphized rather
//! than dispatched through a vtable.
//!
//! The only runtime failure is the sink's I/O. One encoding invariant is also
//! enforced here: a variable-length body must fit the width of its length
//! prefix; a body too long to encode is surfaced as
//! [`std::io::ErrorKind::InvalidData`] rather than silently truncated. (In
//! practice the domain types this framework serializes are bounded well within
//! their prefix widths; the check is defence in depth.)

use std::io::{self, Write};

use super::U24;

/// Builds the "body too long for its length prefix" I/O error.
fn body_too_long(len: usize, prefix_bytes: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("body length {len} exceeds the capacity of a {prefix_bytes}-byte length prefix"),
    )
}

/// Writes a big-endian `uint8`.
///
/// # Errors
///
/// Propagates any I/O error from `writer`.
pub fn write_u8<W: Write>(writer: &mut W, value: u8) -> io::Result<()> {
    writer.write_all(&[value])
}

/// Writes a big-endian `uint16`.
///
/// # Errors
///
/// Propagates any I/O error from `writer`.
pub fn write_u16<W: Write>(writer: &mut W, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

/// Writes a big-endian `uint24`.
///
/// # Errors
///
/// Propagates any I/O error from `writer`.
pub fn write_u24<W: Write>(writer: &mut W, value: U24) -> io::Result<()> {
    // `U24` holds a value `< 2^24`, so the most-significant byte of its 32-bit
    // representation is always zero; the wire encoding is the low three bytes.
    let [_, b1, b2, b3] = value.get().to_be_bytes();
    writer.write_all(&[b1, b2, b3])
}

/// Writes a big-endian `uint32`.
///
/// # Errors
///
/// Propagates any I/O error from `writer`.
pub fn write_u32<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

/// Writes raw bytes (a TLS fixed-size `opaque field[N]` has no length prefix).
///
/// # Errors
///
/// Propagates any I/O error from `writer`.
pub fn write_bytes<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)
}

/// Writes a `u8`-length-prefixed opaque byte string (`opaque v<0..2^8-1>`).
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if `bytes.len()` exceeds `u8::MAX`, plus
/// any I/O error from `writer`.
pub fn write_opaque_u8<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = u8::try_from(bytes.len()).map_err(|_| body_too_long(bytes.len(), 1))?;
    write_u8(writer, len)?;
    writer.write_all(bytes)
}

/// Writes a `u16`-length-prefixed opaque byte string (`opaque v<0..2^16-1>`).
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if `bytes.len()` exceeds `u16::MAX`, plus
/// any I/O error from `writer`.
pub fn write_opaque_u16<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = u16::try_from(bytes.len()).map_err(|_| body_too_long(bytes.len(), 2))?;
    write_u16(writer, len)?;
    writer.write_all(bytes)
}

/// Writes a `u24`-length-prefixed opaque byte string (`opaque v<0..2^24-1>`).
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if `bytes.len()` exceeds `2^24 - 1`, plus
/// any I/O error from `writer`.
pub fn write_opaque_u24<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let len = U24::try_from_usize(bytes.len()).ok_or_else(|| body_too_long(bytes.len(), 3))?;
    write_u24(writer, len)?;
    writer.write_all(bytes)
}

/// Writes a `u8`-length-prefixed vector of serializable items
/// (`T v<0..2^8-1>`).
///
/// Items are serialized into a scratch buffer first so the byte length of the
/// encoded body is known before the prefix is emitted.
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if the encoded body exceeds `u8::MAX`
/// bytes, plus any I/O error from `writer`.
pub fn write_vector_u8<W: Write, T: super::TlsSerialize>(
    writer: &mut W,
    items: &[T],
) -> io::Result<()> {
    let body = serialize_items(items)?;
    write_opaque_u8(writer, &body)
}

/// Writes a `u16`-length-prefixed vector of serializable items
/// (`T v<0..2^16-1>`).
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if the encoded body exceeds `u16::MAX`
/// bytes, plus any I/O error from `writer`.
pub fn write_vector_u16<W: Write, T: super::TlsSerialize>(
    writer: &mut W,
    items: &[T],
) -> io::Result<()> {
    let body = serialize_items(items)?;
    write_opaque_u16(writer, &body)
}

/// Writes a `u24`-length-prefixed vector of serializable items
/// (`T v<0..2^24-1>`).
///
/// # Errors
///
/// [`std::io::ErrorKind::InvalidData`] if the encoded body exceeds `2^24 - 1`
/// bytes, plus any I/O error from `writer`.
pub fn write_vector_u24<W: Write, T: super::TlsSerialize>(
    writer: &mut W,
    items: &[T],
) -> io::Result<()> {
    let body = serialize_items(items)?;
    write_opaque_u24(writer, &body)
}

/// Serializes a slice of items into a contiguous body buffer.
///
/// Writing into a `Vec<u8>` never performs real I/O, so the only error this can
/// surface is an item's own encoding-invariant violation propagated by `?`.
fn serialize_items<T: super::TlsSerialize>(items: &[T]) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    for item in items {
        item.tls_serialize(&mut body)?;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::{
        write_opaque_u16, write_opaque_u8, write_u16, write_u24, write_u32, write_u8,
        write_vector_u16,
    };
    use crate::wire::U24;

    fn to_vec(f: impl FnOnce(&mut Vec<u8>) -> std::io::Result<()>) -> Vec<u8> {
        let mut buf = Vec::new();
        f(&mut buf).expect("writing to a Vec is infallible");
        buf
    }

    #[test]
    fn integers_are_big_endian() {
        assert_eq!(to_vec(|w| write_u8(w, 0x12)), vec![0x12]);
        assert_eq!(to_vec(|w| write_u16(w, 0x0102)), vec![0x01, 0x02]);
        assert_eq!(
            to_vec(|w| write_u24(w, U24::new(0x01_02_03).unwrap())),
            vec![0x01, 0x02, 0x03],
        );
        assert_eq!(
            to_vec(|w| write_u32(w, 0x0102_0304)),
            vec![0x01, 0x02, 0x03, 0x04],
        );
    }

    #[test]
    fn opaque_prefixes_length_then_body() {
        assert_eq!(
            to_vec(|w| write_opaque_u8(w, &[0xAA, 0xBB])),
            vec![0x02, 0xAA, 0xBB],
        );
        assert_eq!(
            to_vec(|w| write_opaque_u16(w, &[0xAA, 0xBB, 0xCC])),
            vec![0x00, 0x03, 0xAA, 0xBB, 0xCC],
        );
        // Empty body still emits the (zero) length prefix.
        assert_eq!(to_vec(|w| write_opaque_u8(w, &[])), vec![0x00]);
    }

    #[test]
    fn typed_vector_prefixes_encoded_body_length() {
        // Three u16 items -> 6-byte body, u16 length prefix 0x0006.
        assert_eq!(
            to_vec(|w| write_vector_u16(w, &[0x0001u16, 0x0002, 0x0003])),
            vec![0x00, 0x06, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03],
        );
    }

    #[test]
    fn opaque_body_too_long_for_prefix_is_invalid_data_not_truncation() {
        // 256 bytes cannot be described by a u8 length prefix; expect an error
        // rather than a silently wrapped length.
        let body = vec![0u8; 256];
        let mut buf = Vec::new();
        let err = write_opaque_u8(&mut buf, &body).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        // Nothing partial was committed past the failed length encode.
        assert!(buf.is_empty());
    }
}
