//! Signs and verifies a sample payload with a fresh ECDSA P-256 key.
//!
//! Demo for ticket `mtclib-signing`:
//! `cargo run -p mtc --example sign_and_verify`.

use mtc::signing::{EcdsaP256, SignatureScheme};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scheme = EcdsaP256;

    // Production keys live in the HSM (spec §14); here we mint a fresh one.
    let (signing_key, verifying_key) = EcdsaP256::generate_keypair();

    let payload = b"checkpoint: log=demo tree_size=1000 root=<32-byte hash>";
    let signature = scheme.sign(&signing_key, payload)?;

    println!("algorithm:     {}", scheme.algorithm());
    println!("codepoint:     {:#06x}", scheme.algorithm().code());
    println!(
        "public key:    {} bytes of SPKI DER",
        verifying_key.spki_der().len()
    );
    println!("signature:     {} bytes (P1363 r||s)", signature.len());

    scheme.verify(&verifying_key, payload, &signature)?;
    println!("verification:  OK");

    // A tampered payload must not verify.
    match scheme.verify(&verifying_key, b"tampered payload", &signature) {
        Ok(()) => return Err("tampered payload verified — this must never happen".into()),
        Err(err) => println!("tampered:      rejected ({err})"),
    }

    Ok(())
}
