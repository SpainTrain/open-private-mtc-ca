//! Builds, signs, serializes, re-parses, and verifies a checkpoint.
//!
//! Demo for ticket `mtclib-checkpoint`:
//! `cargo run -p mtc --example checkpoint_demo`.

use mtc::checkpoint::{Checkpoint, CheckpointBuilder, Signed};
use mtc::signing::EcdsaP256;
use mtc::{HashOutput, LogId, SignedAt, TreeSize, SHA256_EMPTY_ROOT};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scheme = EcdsaP256;

    // Production checkpoint signing runs on the HSM (spec §14); here we mint a
    // fresh ECDSA P-256 keypair for the demo.
    let (signing_key, verifying_key) = EcdsaP256::generate_keypair();

    // Build an *unsigned* checkpoint via the §22.2 typestate builder. Omitting
    // any of the four fields would be a compile error, not a runtime one.
    let unsigned = CheckpointBuilder::new(LogId::new("demo-ca")?)
        .root_hash(HashOutput([0xab; 32]))
        .tree_size(TreeSize(1000))
        .signed_at(SignedAt(1_700_000_000))
        .build();

    println!("log id:        {}", unsigned.log_id());
    println!("tree size:     {}", unsigned.tree_size().0);
    println!(
        "signed input:  {} bytes (MTCSubtreeSignatureInput, draft §5.4.1)",
        unsigned.signature_input()?.len()
    );

    // Sign it: this transitions Checkpoint<Unsigned> -> Checkpoint<Signed>. An
    // unsigned checkpoint has no `verify` method (spec §22.4).
    let signed = unsigned.sign(&scheme, &signing_key)?;
    println!(
        "signature:     {} bytes (P1363 r||s)",
        signed.signature().len()
    );

    // Serialize to the TLS-presentation wire form, then parse it straight back.
    let bytes = signed.serialize_tls_presentation()?;
    println!("wire form:     {} bytes", bytes.len());
    let parsed = Checkpoint::<Signed>::parse_tls_presentation(&bytes)?;
    assert_eq!(signed, parsed, "round-trip: parse(serialize(x)) == x");
    println!("round-trip:    OK");

    // Verify the re-parsed checkpoint's signature.
    parsed.verify(&scheme, &verifying_key)?;
    println!("verification:  OK");

    // Tamper with the committed tree_size on the wire, re-parse, and confirm the
    // original signature no longer verifies (it covered the original size).
    let mut mutated = bytes;
    let tree_size_offset = 1 + "demo-ca".len(); // after the TrustAnchorID(log_id)
    mutated[tree_size_offset + 7] ^= 0x01; // flip the low byte of the u64
    let tampered = Checkpoint::<Signed>::parse_tls_presentation(&mutated)?;
    match tampered.verify(&scheme, &verifying_key) {
        Ok(()) => return Err("a mutated tree_size verified — this must never happen".into()),
        Err(err) => println!("tampered size: rejected ({err})"),
    }

    // Empty-tree checkpoint: well-defined and constant-rooted (spec §19.6).
    let empty = CheckpointBuilder::new(LogId::new("demo-ca")?)
        .root_hash(SHA256_EMPTY_ROOT)
        .tree_size(TreeSize(0))
        .signed_at(SignedAt(1_700_000_000))
        .build()
        .sign(&scheme, &signing_key)?;
    empty.verify(&scheme, &verifying_key)?;
    println!("empty tree:    OK (root {:?})", empty.root_hash());

    Ok(())
}
