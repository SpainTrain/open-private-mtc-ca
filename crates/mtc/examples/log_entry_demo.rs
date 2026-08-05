//! Log-entry demo (ticket `mtclib-log-entries`).
//!
//! Builds a sample `TbsCertificateLogEntry` that commits to the **hash** of a
//! subject public key (never the raw key), wraps it as a log leaf, and prints
//! the hex leaf hash of both the sample entry and the spec-defined `null_entry`
//! placeholder. Also prints `null_entry`'s exact wire bytes, which are locked by
//! a unit test.
//!
//! ```console
//! $ cargo run -p mtc --example log_entry_demo
//! ```

use std::net::Ipv4Addr;

use mtc::{
    null_entry, Claim, DnsName, LogEntry, Sha256Hasher, SubjectType, TbsCertificateLogEntry,
    TlsSerialize, TlsSubjectInfo,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A subject's TLS public key (a placeholder 65-byte uncompressed EC point
    // shape) with its IANA SignatureScheme codepoint. The log entry commits to
    // HASH(subject_info), so the raw key never enters the entry (spec §2).
    let subject_info = TlsSubjectInfo::new(0x0403, vec![0x04; 65])?;
    let subject_info_hash = subject_info.subject_info_hash::<Sha256Hasher>()?;

    // Typestate builder: `.build()` is reachable only with both required fields.
    let entry = TbsCertificateLogEntry::builder()
        .subject_type(SubjectType::Tls)
        .subject_info_hash(subject_info_hash)
        .claim(Claim::dns(vec![DnsName::new(b"example.com".to_vec())?])?)
        .claim(Claim::ipv4(vec![Ipv4Addr::new(192, 0, 2, 1)])?)
        .build();

    let claim_count = entry.claims().len();
    let cert = LogEntry::certificate(entry);

    println!("sample entry claims    : {claim_count} claim(s)");
    println!(
        "sample entry leaf hash : {:?}",
        cert.leaf_hash::<Sha256Hasher>()?
    );
    println!(
        "null_entry wire bytes  : {:02x?}",
        null_entry().tls_serialize_to_vec()?
    );
    println!(
        "null_entry leaf hash   : {:?}",
        null_entry().leaf_hash::<Sha256Hasher>()?
    );

    Ok(())
}
