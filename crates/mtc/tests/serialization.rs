//! Demo + acceptance tests for the TLS-presentation serialization framework
//! (ticket `mtc-serialization`, spec sections 19.2 and 19.3 layer 1).
//!
//! Run by the ticket's demo command `cargo test -p mtc serialization`; every
//! test lives under the `serialization` module so the name filter selects them.
//! Coverage: round-trip identity with pinned known-answer byte vectors, malformed
//! and adversarial input rejection, and a property test asserting that parsing
//! arbitrary byte strings never panics.

// Test-only crate: `unwrap`/`expect` are the ergonomic choice here and the
// production ban does not apply to test code (docs/lint-policy.md notes that
// non-`#[test]` helpers in integration-test files still need this scoped allow).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod serialization {
    use mtc::wire::{write_opaque_u16, write_vector_u16, TlsReader};
    use mtc::{assert_roundtrip, TlsParse, TlsSerialize, WireError, U24};
    use proptest::prelude::*;

    #[test]
    fn serialization_round_trip_with_known_answer_vectors() {
        // Byte formats are fixed by the presentation language (RFC 8446 §3):
        // big-endian integers, fixed opaque without a length prefix.
        assert_roundtrip!(0x12_u8, [0x12]);
        assert_roundtrip!(0x0102_u16, [0x01, 0x02]);
        assert_roundtrip!(U24::new(0x0001_0203).unwrap(), [0x01, 0x02, 0x03]);
        assert_roundtrip!(0xDEAD_BEEF_u32, [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_roundtrip!([0xAA_u8, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn serialization_length_prefixed_forms_round_trip() {
        // opaque<0..2^16-1>: u16 length prefix then body.
        let body = [0x01_u8, 0x02, 0x03];
        let mut buf = Vec::new();
        write_opaque_u16(&mut buf, &body).unwrap();
        assert_eq!(buf, vec![0x00, 0x03, 0x01, 0x02, 0x03]);
        let mut reader = TlsReader::new(&buf);
        assert_eq!(reader.read_opaque_u16().unwrap(), &body[..]);
        assert!(reader.finish().is_ok());

        // Vec<u16><0..2^16-1>: prefix is the encoded *byte* length (6), not the
        // element count (3).
        let mut buf = Vec::new();
        write_vector_u16(&mut buf, &[0x0001_u16, 0x0002, 0x0003]).unwrap();
        assert_eq!(buf, vec![0x00, 0x06, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03]);
        let mut reader = TlsReader::new(&buf);
        let items: Vec<u16> = reader.read_vector_u16().unwrap();
        assert_eq!(items, vec![0x0001, 0x0002, 0x0003]);
    }

    #[test]
    fn serialization_rejects_impossible_length_claim() {
        // u16 prefix claims 0xFFFF bytes but the body is empty: refused before
        // any 65 535-byte allocation is sized (spec §19.3).
        let mut reader = TlsReader::new(&[0xFF, 0xFF]);
        assert_eq!(
            reader.read_opaque_u16(),
            Err(WireError::LengthOverflow {
                offset: 2,
                claimed: 0xFFFF,
                remaining: 0,
            }),
        );
    }

    #[test]
    fn serialization_rejects_truncated_input() {
        // A u32 needs four bytes; one byte yields EOF, never a panic.
        assert_eq!(
            u32::tls_parse_exact(&[0xAB]),
            Err(WireError::UnexpectedEof {
                offset: 0,
                needed: 4,
                remaining: 1,
            }),
        );
    }

    #[test]
    fn serialization_rejects_trailing_bytes() {
        // One byte too many after a well-formed u16 is rejected, not ignored.
        assert_eq!(
            u16::tls_parse_exact(&[0x01, 0x02, 0x03]),
            Err(WireError::TrailingBytes {
                offset: 2,
                remaining: 1,
            }),
        );
    }

    proptest! {
        /// Spec §19.3 asserted property: no panic on any input. Every parse
        /// entry point returns `Ok`/`Err` for arbitrary bytes — a panic (index
        /// out of bounds, overflow, unwrap) would surface as a test failure.
        #[test]
        fn serialization_arbitrary_bytes_never_panic(
            data in prop::collection::vec(any::<u8>(), 0..4096),
        ) {
            let _ = u8::tls_parse_exact(&data);
            let _ = u16::tls_parse_exact(&data);
            let _ = u32::tls_parse_exact(&data);
            let _ = U24::tls_parse_exact(&data);
            let _ = <[u8; 16]>::tls_parse_exact(&data);

            // Length-prefixed forms drive the allocation-bounding and
            // depth/loop guards.
            let _ = TlsReader::new(&data).read_opaque_u8();
            let _ = TlsReader::new(&data).read_opaque_u16();
            let _ = TlsReader::new(&data).read_opaque_u24();
            let _: Result<Vec<u16>, _> = TlsReader::new(&data).read_vector_u16();
            let _: Result<Vec<u32>, _> = TlsReader::new(&data).read_vector_u24();
        }

        /// Spec §19.2: `parse(serialize(x)) == x` for the integer primitives.
        #[test]
        fn serialization_integer_round_trips(v in any::<u32>()) {
            prop_assert_eq!(u32::tls_parse_exact(&v.tls_serialize_to_vec()), Ok(v));
            #[allow(clippy::cast_possible_truncation)]
            let narrow = v as u16;
            prop_assert_eq!(u16::tls_parse_exact(&narrow.tls_serialize_to_vec()), Ok(narrow));
        }

        /// Round-trip for `uint24` across its full range.
        #[test]
        fn serialization_u24_round_trips(v in 0u32..=0x00FF_FFFF) {
            let u = U24::new(v).unwrap();
            prop_assert_eq!(U24::tls_parse_exact(&u.tls_serialize_to_vec()), Ok(u));
        }

        /// Round-trip for length-prefixed opaque strings of arbitrary content.
        #[test]
        fn serialization_opaque_round_trips(
            body in prop::collection::vec(any::<u8>(), 0..2048),
        ) {
            let mut buf = Vec::new();
            write_opaque_u16(&mut buf, &body).unwrap();
            let mut reader = TlsReader::new(&buf);
            prop_assert_eq!(reader.read_opaque_u16().unwrap(), &body[..]);
            prop_assert!(reader.finish().is_ok());
        }

        /// Round-trip for length-prefixed vectors of typed elements.
        #[test]
        fn serialization_vector_round_trips(
            items in prop::collection::vec(any::<u32>(), 0..512),
        ) {
            let mut buf = Vec::new();
            write_vector_u16(&mut buf, &items).unwrap();
            let mut reader = TlsReader::new(&buf);
            let parsed: Vec<u32> = reader.read_vector_u16().unwrap();
            prop_assert_eq!(parsed, items);
        }
    }
}
