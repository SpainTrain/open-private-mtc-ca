//! Construction-time errors for log-entry claim and subject types (spec
//! section 22.6; rule `thiserror-for-libs-eyre-for-bins`).
//!
//! These are the *builder-side* errors: they fire when in-memory library code
//! tries to construct a value that violates a draft `<min..max>` bound (an
//! empty DNS name, an empty claim list, an empty public key). The *parser-side*
//! counterpart for the same constraints on untrusted bytes is
//! [`WireError::InvalidValue`](crate::wire::WireError::InvalidValue); a codec
//! that reads such a value maps this error onto that variant. Keeping the two
//! separate keeps construction failures out of the wire error surface and vice
//! versa.

use thiserror::Error;

/// Reasons a claim, subject, or entry field was rejected at construction.
///
/// Every variant corresponds to a TLS-presentation `<min..max>` floor from
/// `draft-ietf-plants-merkle-tree-certs-03` that the generic wire framework
/// does not enforce (it polices only upper bounds); the crypto finding F3 /
/// bead `mtc-qka.3` minimum-length requirement is realized here for the
/// construction path and in the codecs for the parse path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum EntryError {
    /// A DNS name was empty. `DNSName` is `opaque<1..255>`
    /// (draft-03 §4.1): at least one byte is required.
    #[error("DNS name must be 1..=255 bytes, got 0")]
    EmptyDnsName,

    /// A DNS name exceeded 255 bytes. `DNSName` is `opaque<1..255>`
    /// (draft-03 §4.1): the `u8` length prefix caps it at 255.
    #[error("DNS name must be 1..=255 bytes, got {actual}")]
    DnsNameTooLong {
        /// The over-long name's length in bytes.
        actual: usize,
    },

    /// A claim's value list was empty. The DNS and IP address lists are
    /// non-empty vectors (`dns_names<1..2^16-1>`, `addresses<4..2^16-1>` /
    /// `<16..2^16-1>`; draft-03 §4.1, §4.2): a claim asserting nothing is not
    /// representable.
    #[error("claim value list must contain at least one entry")]
    EmptyClaimList,

    /// A subject public key was empty. `public_key` is `opaque<1..2^16-1>`
    /// (draft-03 §4, TLS subject info): at least one byte is required.
    #[error("subject public key must be at least one byte")]
    EmptyPublicKey,
}
