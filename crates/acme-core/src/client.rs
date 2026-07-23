//! Client-side ACME JWS construction.
//!
//! Deliberately minimal: enough for tests, the fuzz corpus, the in-process
//! integration client, and the scripted demo (`examples/demo_client.rs`).
//! Not a general-purpose ACME client.

use base64ct::{Base64UrlUnpadded, Encoding};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};

use crate::error::{AcmeError, ES256};
use crate::jws::Jwk;

/// How the request binds to the account key (RFC 8555 §6.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClientBinding {
    /// Inline `jwk` (used for `new-account`).
    Jwk,
    /// `kid` account URL (used for authenticated requests after
    /// registration).
    Kid(String),
}

/// Builds a flattened-JSON JWS request body signed with `key` (ES256).
///
/// # Errors
/// [`AcmeError::BadPublicKey`] if the key encodes to the identity point
/// (never for a valid `SigningKey`); [`AcmeError::Internal`] if JSON
/// serialization fails.
pub fn signed_request_body(
    key: &SigningKey,
    binding: &ClientBinding,
    nonce: &str,
    url: &str,
    payload: &serde_json::Value,
) -> Result<String, AcmeError> {
    let mut protected = serde_json::json!({
        "alg": ES256,
        "nonce": nonce,
        "url": url,
    });
    match binding {
        ClientBinding::Jwk => {
            let jwk = Jwk::from_verifying_key(&VerifyingKey::from(key))?;
            protected["jwk"] = serde_json::to_value(&jwk)
                .map_err(|e| AcmeError::Internal(format!("jwk serialization: {e}")))?;
        }
        ClientBinding::Kid(kid) => {
            protected["kid"] = serde_json::Value::String(kid.clone());
        }
    }
    let protected_b64 = Base64UrlUnpadded::encode_string(protected.to_string().as_bytes());
    let payload_b64 = Base64UrlUnpadded::encode_string(payload.to_string().as_bytes());
    let signing_input = format!("{protected_b64}.{payload_b64}");
    let signature: Signature = key.sign(signing_input.as_bytes());
    let body = serde_json::json!({
        "protected": protected_b64,
        "payload": payload_b64,
        "signature": Base64UrlUnpadded::encode_string(&signature.to_bytes()),
    });
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jws::{AccountKeySource, Jws};

    #[test]
    fn produced_body_parses_and_verifies() {
        let key = SigningKey::from_slice(&[9; 32]).expect("valid scalar");
        let body = signed_request_body(
            &key,
            &ClientBinding::Jwk,
            "nonce-1",
            "http://localhost/acme/new-account",
            &serde_json::json!({"termsOfServiceAgreed": true}),
        )
        .expect("signable");
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        jws.check_alg().expect("ES256");
        let AccountKeySource::Jwk(jwk) = jws.account_key().expect("binding") else {
            panic!("expected jwk binding");
        };
        jws.verify_signature(&jwk.verifying_key().expect("key"))
            .expect("verifies");
    }

    #[test]
    fn kid_binding_omits_jwk() {
        let key = SigningKey::from_slice(&[9; 32]).expect("valid scalar");
        let body = signed_request_body(
            &key,
            &ClientBinding::Kid("http://localhost/acme/acct/1".into()),
            "nonce-1",
            "http://localhost/acme/whatever",
            &serde_json::json!({}),
        )
        .expect("signable");
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert_eq!(
            jws.account_key().expect("binding"),
            AccountKeySource::Kid("http://localhost/acme/acct/1")
        );
    }
}
