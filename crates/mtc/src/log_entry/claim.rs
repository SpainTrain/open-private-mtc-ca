//! Assertion claims: the DNS-name and IP-address identifiers a log entry
//! asserts (`draft-ietf-plants-merkle-tree-certs-03` §4; spec §2 concept
//! "claims").
//!
//! A `Claim` is `{ ClaimType claim_type; opaque claim_info<0..2^16-1>; }`
//! (draft-03 §4). The `claim_info` opaque wrapper lets a reader that does not
//! understand a claim type skip it; a reader that *does* understand it must
//! consume the wrapped body exactly. The v1 claim types and their payloads:
//!
//! | `ClaimType`      | code | `claim_info` body                          |
//! |------------------|------|--------------------------------------------|
//! | `dns`            | 0    | `DNSName dns_names<1..2^16-1>` (§4.1)       |
//! | `dns_wildcard`   | 1    | `DNSName dns_names<1..2^16-1>` (§4.1)       |
//! | `ipv4`           | 2    | `IPv4Address addresses<4..2^16-1>` (§4.2)   |
//! | `ipv6`           | 3    | `IPv6Address addresses<16..2^16-1>` (§4.2)  |
//!
//! where `DNSName` is `opaque<1..255>`, `IPv4Address` is `uint8[4]`, and
//! `IPv6Address` is `uint8[16]`. Every list is non-empty and every `DNSName` is
//! non-empty: those `<min..>` floors are the hand-enforced minimum-length
//! fields of crypto finding F3 / bead `mtc-qka.3`, checked here on both the
//! construction and the parse path (the generic wire reader polices only upper
//! bounds).

use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::wire::{
    write_opaque_u16, write_opaque_u8, write_u16, write_vector_u16, TlsParse, TlsReader,
    TlsSerialize, WireError,
};

use super::error::EntryError;

/// A DNS name as it appears in a `dns` / `dns_wildcard` claim: `opaque
/// DNSName<1..255>` (draft-03 §4.1).
///
/// Stored as raw bytes (DNS names are case-insensitive ASCII / A-labels; the
/// draft treats them as opaque). The `1..255` length bound is enforced at
/// construction, so a `DnsName` value is always a valid wire `DNSName`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DnsName(Vec<u8>);

impl DnsName {
    /// The maximum length of a `DNSName` in bytes (the `opaque<1..255>` ceiling
    /// imposed by its `u8` length prefix).
    pub const MAX_LEN: usize = 255;

    /// Creates a DNS name from bytes, enforcing the `1..=255` length bound.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyDnsName`] if empty; [`EntryError::DnsNameTooLong`] if
    /// longer than [`Self::MAX_LEN`].
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, EntryError> {
        let bytes = bytes.into();
        match bytes.len() {
            0 => Err(EntryError::EmptyDnsName),
            n if n > Self::MAX_LEN => Err(EntryError::DnsNameTooLong { actual: n }),
            _ => Ok(Self(bytes)),
        }
    }

    /// Borrows the raw name bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TlsSerialize for DnsName {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // opaque<1..255>: the `1..` floor holds by construction; `write_opaque_u8`
        // enforces the 255 ceiling against its `u8` prefix.
        write_opaque_u8(writer, &self.0)
    }
}

impl TlsParse for DnsName {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let bytes = reader.read_opaque_u8()?;
        // Hand-enforced `DNSName<1..255>` floor (crypto F3). The `u8` prefix
        // already bounds the ceiling; only the empty case can reach here.
        Self::new(bytes).map_err(|_| WireError::InvalidValue {
            offset: reader.position(),
            reason: "DNSName must be 1..=255 bytes",
        })
    }
}

/// The type tag of a [`Claim`] (`ClaimType` enum, draft-03 §4).
///
/// A closed set (spec §22.3) encoded as a `uint16` (the draft's `(2^16-1)`
/// maximum). An unrecognized value parses to [`WireError::InvalidValue`], never
/// a panic (spec §22.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClaimType {
    /// `dns(0)`: a list of DNS names (draft-03 §4.1).
    Dns,
    /// `dns_wildcard(1)`: a list of wildcard DNS names (draft-03 §4.1).
    DnsWildcard,
    /// `ipv4(2)`: a list of IPv4 addresses (draft-03 §4.2).
    Ipv4,
    /// `ipv6(3)`: a list of IPv6 addresses (draft-03 §4.2).
    Ipv6,
}

impl ClaimType {
    /// The on-wire `uint16` codepoint for this claim type.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Dns => 0,
            Self::DnsWildcard => 1,
            Self::Ipv4 => 2,
            Self::Ipv6 => 3,
        }
    }

    /// Resolves a `uint16` codepoint to a claim type, or `None` if unknown.
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            0 => Some(Self::Dns),
            1 => Some(Self::DnsWildcard),
            2 => Some(Self::Ipv4),
            3 => Some(Self::Ipv6),
            _ => None,
        }
    }
}

/// One claim asserted by a log entry: a non-empty set of identifiers of a
/// single [`ClaimType`] (draft-03 §4; spec §2 "claims").
///
/// Each variant's list is non-empty by construction — the draft's
/// `dns_names<1..2^16-1>` / `addresses<4..2^16-1>` / `<16..2^16-1>` floors
/// (crypto F3 / bead `mtc-qka.3`). Build one through the checked constructors;
/// the parser applies the same floors to untrusted bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Claim {
    /// A `dns` claim: one or more DNS names.
    Dns(Vec<DnsName>),
    /// A `dns_wildcard` claim: one or more wildcard DNS names.
    DnsWildcard(Vec<DnsName>),
    /// An `ipv4` claim: one or more IPv4 addresses.
    Ipv4(Vec<Ipv4Addr>),
    /// An `ipv6` claim: one or more IPv6 addresses.
    Ipv6(Vec<Ipv6Addr>),
}

impl Claim {
    /// Creates a `dns` claim from a non-empty list of names.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyClaimList`] if `names` is empty.
    pub fn dns(names: impl Into<Vec<DnsName>>) -> Result<Self, EntryError> {
        let names = non_empty(names.into())?;
        Ok(Self::Dns(names))
    }

    /// Creates a `dns_wildcard` claim from a non-empty list of names.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyClaimList`] if `names` is empty.
    pub fn dns_wildcard(names: impl Into<Vec<DnsName>>) -> Result<Self, EntryError> {
        let names = non_empty(names.into())?;
        Ok(Self::DnsWildcard(names))
    }

    /// Creates an `ipv4` claim from a non-empty list of addresses.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyClaimList`] if `addresses` is empty.
    pub fn ipv4(addresses: impl Into<Vec<Ipv4Addr>>) -> Result<Self, EntryError> {
        let addresses = non_empty(addresses.into())?;
        Ok(Self::Ipv4(addresses))
    }

    /// Creates an `ipv6` claim from a non-empty list of addresses.
    ///
    /// # Errors
    ///
    /// [`EntryError::EmptyClaimList`] if `addresses` is empty.
    pub fn ipv6(addresses: impl Into<Vec<Ipv6Addr>>) -> Result<Self, EntryError> {
        let addresses = non_empty(addresses.into())?;
        Ok(Self::Ipv6(addresses))
    }

    /// The type tag of this claim.
    #[must_use]
    pub const fn claim_type(&self) -> ClaimType {
        match self {
            Self::Dns(_) => ClaimType::Dns,
            Self::DnsWildcard(_) => ClaimType::DnsWildcard,
            Self::Ipv4(_) => ClaimType::Ipv4,
            Self::Ipv6(_) => ClaimType::Ipv6,
        }
    }

    /// Serializes just the `claim_info` body (the inner list) to a buffer.
    fn serialize_claim_info(&self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        match self {
            Self::Dns(names) | Self::DnsWildcard(names) => {
                write_vector_u16(&mut body, names)?;
            }
            Self::Ipv4(addresses) => {
                let raw: Vec<[u8; 4]> = addresses.iter().map(Ipv4Addr::octets).collect();
                write_vector_u16(&mut body, &raw)?;
            }
            Self::Ipv6(addresses) => {
                let raw: Vec<[u8; 16]> = addresses.iter().map(Ipv6Addr::octets).collect();
                write_vector_u16(&mut body, &raw)?;
            }
        }
        Ok(body)
    }
}

impl TlsSerialize for Claim {
    fn tls_serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        write_u16(writer, self.claim_type().code())?;
        // The inner list is wrapped in the generic `claim_info<0..2^16-1>`
        // opaque so an unknown claim type is skippable (draft-03 §4).
        let body = self.serialize_claim_info()?;
        write_opaque_u16(writer, &body)
    }
}

impl TlsParse for Claim {
    fn tls_parse(reader: &mut TlsReader<'_>) -> Result<Self, WireError> {
        let code = reader.read_u16()?;
        let claim_type = ClaimType::from_code(code).ok_or_else(|| WireError::InvalidValue {
            offset: reader.position(),
            reason: "unknown ClaimType codepoint",
        })?;
        let body = reader.read_opaque_u16()?;

        // Parse the wrapped list from a reader bounded to `claim_info`, then
        // require it to be fully consumed: a known claim type must match its
        // wrapper length exactly (trailing bytes inside `claim_info` are
        // rejected). No claim nests another claim, so a fresh reader here cannot
        // be used to bypass the outer depth budget.
        let mut sub = TlsReader::new(body);
        let claim = match claim_type {
            ClaimType::Dns => Self::Dns(parse_name_list(&mut sub)?),
            ClaimType::DnsWildcard => Self::DnsWildcard(parse_name_list(&mut sub)?),
            ClaimType::Ipv4 => {
                let raw: Vec<[u8; 4]> = non_empty_wire(sub.read_vector_u16()?, &sub)?;
                Self::Ipv4(raw.into_iter().map(Ipv4Addr::from).collect())
            }
            ClaimType::Ipv6 => {
                let raw: Vec<[u8; 16]> = non_empty_wire(sub.read_vector_u16()?, &sub)?;
                Self::Ipv6(raw.into_iter().map(Ipv6Addr::from).collect())
            }
        };
        sub.finish()?;
        Ok(claim)
    }
}

/// Parses a non-empty `DNSName dns_names<1..2^16-1>` list from `sub`.
fn parse_name_list(sub: &mut TlsReader<'_>) -> Result<Vec<DnsName>, WireError> {
    non_empty_wire(sub.read_vector_u16()?, sub)
}

/// Rejects an empty in-memory claim list (construction path).
fn non_empty<T>(items: Vec<T>) -> Result<Vec<T>, EntryError> {
    if items.is_empty() {
        Err(EntryError::EmptyClaimList)
    } else {
        Ok(items)
    }
}

/// Rejects an empty parsed claim list (parse path): the `<1..>` / `<4..>` /
/// `<16..>` floor the generic vector reader does not enforce (crypto F3).
fn non_empty_wire<T>(items: Vec<T>, sub: &TlsReader<'_>) -> Result<Vec<T>, WireError> {
    if items.is_empty() {
        Err(WireError::InvalidValue {
            offset: sub.position(),
            reason: "claim value list must contain at least one entry",
        })
    } else {
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::{Claim, ClaimType, DnsName, EntryError};
    use crate::assert_roundtrip;
    use crate::wire::{TlsParse, WireError};

    fn dns_name(s: &str) -> DnsName {
        DnsName::new(s.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn claim_type_codepoints() {
        for (ct, code) in [
            (ClaimType::Dns, 0u16),
            (ClaimType::DnsWildcard, 1),
            (ClaimType::Ipv4, 2),
            (ClaimType::Ipv6, 3),
        ] {
            assert_eq!(ct.code(), code);
            assert_eq!(ClaimType::from_code(code), Some(ct));
        }
        assert_eq!(ClaimType::from_code(4), None);
        assert_eq!(ClaimType::from_code(0xFFFF), None);
    }

    #[test]
    fn dns_name_enforces_length_bounds() {
        assert_eq!(
            DnsName::new(Vec::new()).unwrap_err(),
            EntryError::EmptyDnsName
        );
        assert_eq!(
            DnsName::new(vec![b'a'; 256]).unwrap_err(),
            EntryError::DnsNameTooLong { actual: 256 },
        );
        assert!(DnsName::new(vec![b'a'; 255]).is_ok());
        assert_eq!(dns_name("example.com").as_bytes(), b"example.com");
    }

    #[test]
    fn dns_claim_round_trips_and_pins_bytes() {
        let claim = Claim::dns(vec![dns_name("a.example")]).unwrap();
        // "a.example" is 9 bytes -> DNSName = [0x09] + 9 = 10 bytes;
        // dns_names vector body = 10 (0x0A); claim_info = 2-byte vec prefix + 10
        // = 12 (0x0C).
        let expected = [
            0x00, 0x00, // ClaimType::dns
            0x00, 0x0C, // claim_info length (2 vec prefix + 10 vec body)
            0x00, 0x0A, // dns_names vector byte length (1 len + 9 name)
            0x09, // DNSName length
            b'a', b'.', b'e', b'x', b'a', b'm', b'p', b'l', b'e',
        ];
        assert_roundtrip!(claim, expected);
    }

    #[test]
    fn dns_wildcard_claim_round_trips() {
        let claim =
            Claim::dns_wildcard(vec![dns_name("*.example.com"), dns_name("*.test")]).unwrap();
        assert_roundtrip!(claim);
    }

    #[test]
    fn ipv4_claim_round_trips_and_pins_bytes() {
        let claim = Claim::ipv4(vec![Ipv4Addr::new(192, 0, 2, 1)]).unwrap();
        let expected = [
            0x00, 0x02, // ClaimType::ipv4
            0x00, 0x06, // claim_info length (2 + 4)
            0x00, 0x04, // addresses vector byte length
            192, 0, 2, 1, // the address
        ];
        assert_roundtrip!(claim, expected);
    }

    #[test]
    fn ipv6_claim_round_trips() {
        let claim = Claim::ipv6(vec![
            Ipv6Addr::LOCALHOST,
            Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1),
        ])
        .unwrap();
        assert_roundtrip!(claim);
    }

    #[test]
    fn empty_claim_lists_are_rejected_on_construction() {
        assert_eq!(
            Claim::dns(Vec::new()).unwrap_err(),
            EntryError::EmptyClaimList
        );
        assert_eq!(
            Claim::ipv4(Vec::new()).unwrap_err(),
            EntryError::EmptyClaimList
        );
    }

    #[test]
    fn empty_dns_name_list_is_rejected_on_parse() {
        // type=dns; claim_info len=2; dns_names vec len=0 (empty) -> below the
        // <1..> floor.
        let bytes = [0x00, 0x00, 0x00, 0x02, 0x00, 0x00];
        let parsed = Claim::tls_parse_exact(&bytes);
        assert!(
            matches!(
                parsed,
                Err(WireError::InvalidValue {
                    reason: "claim value list must contain at least one entry",
                    ..
                })
            ),
            "{parsed:?}"
        );
    }

    #[test]
    fn empty_dns_name_is_rejected_on_parse() {
        // dns claim with one zero-length DNSName -> below DNSName<1..> floor.
        let bytes = [
            0x00, 0x00, // dns
            0x00, 0x03, // claim_info len
            0x00, 0x01, // dns_names vec byte len
            0x00, // DNSName length = 0 (empty)
        ];
        let parsed = Claim::tls_parse_exact(&bytes);
        assert!(
            matches!(
                parsed,
                Err(WireError::InvalidValue {
                    reason: "DNSName must be 1..=255 bytes",
                    ..
                })
            ),
            "{parsed:?}"
        );
    }

    #[test]
    fn unknown_claim_type_is_rejected_not_panicked() {
        let bytes = [0x00, 0x09, 0x00, 0x00];
        let parsed = Claim::tls_parse_exact(&bytes);
        assert!(
            matches!(
                parsed,
                Err(WireError::InvalidValue {
                    reason: "unknown ClaimType codepoint",
                    ..
                })
            ),
            "{parsed:?}"
        );
    }

    #[test]
    fn misaligned_ipv4_body_is_rejected_not_panicked() {
        // addresses vector byte length 3: not a whole 4-byte address.
        let bytes = [
            0x00, 0x02, // ipv4
            0x00, 0x05, // claim_info len (2 + 3)
            0x00, 0x03, // addresses vec byte len = 3 (misaligned)
            192, 0, 2,
        ];
        let parsed = Claim::tls_parse_exact(&bytes);
        assert!(parsed.is_err(), "{parsed:?}");
    }

    #[test]
    fn trailing_bytes_inside_claim_info_are_rejected() {
        // A valid single-address ipv4 body plus one extra byte inside claim_info.
        let bytes = [
            0x00, 0x02, // ipv4
            0x00, 0x07, // claim_info len (2 + 4 + 1 extra)
            0x00, 0x04, // addresses vec byte len
            192, 0, 2, 1,    // address
            0xFF, // stray trailing byte inside claim_info
        ];
        let parsed = Claim::tls_parse_exact(&bytes);
        assert!(
            matches!(parsed, Err(WireError::TrailingBytes { .. })),
            "{parsed:?}"
        );
    }
}
