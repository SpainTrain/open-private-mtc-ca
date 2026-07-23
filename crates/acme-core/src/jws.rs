//! ACME JWS parsing and ES256 verification (RFC 8555 §6.2, RFC 7515).
//!
//! ACME requests are flattened-JSON JWS objects. Parsing
//! ([`Jws::parse`]) is pure and total over arbitrary bytes — it never
//! panics, which the fuzz target (`fuzz/fuzz_targets/fuzz_jws_parse.rs`,
//! spec §19.3) and a proptest harness both assert. Verification is split
//! into explicit checks (`alg`, nonce, `url`, key source, signature) so
//! handlers can map each failure to the right problem document.

use std::fmt;

use base64ct::{Base64UrlUnpadded, Encoding};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AcmeError, ES256};
use crate::nonce::Nonce;

/// RFC 7638 JWK thumbprint: base64url(SHA-256(canonical-JWK)).
///
/// This is the account-store key: an ACME account *is* its key.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct JwkThumbprint(String);

impl JwkThumbprint {
    /// Base64url thumbprint string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JwkThumbprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An EC public JWK (RFC 7517/7518), restricted to what ACME ES256 needs.
///
/// Extra members (`use`, `alg`, ...) are ignored on input; the required
/// members alone define the RFC 7638 thumbprint.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Jwk {
    /// Key type; must be `"EC"`.
    pub kty: String,
    /// Curve; must be `"P-256"`.
    pub crv: String,
    /// Base64url-encoded X coordinate (32 bytes).
    pub x: String,
    /// Base64url-encoded Y coordinate (32 bytes).
    pub y: String,
}

impl Jwk {
    /// Encodes a P-256 verifying key as a JWK (client side: tests, examples,
    /// demo scripts).
    ///
    /// # Errors
    /// [`AcmeError::BadPublicKey`] only for the identity point, which no
    /// valid key produces.
    pub fn from_verifying_key(key: &VerifyingKey) -> Result<Self, AcmeError> {
        let point = key.to_encoded_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            return Err(AcmeError::BadPublicKey("identity point".into()));
        };
        Ok(Self {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: Base64UrlUnpadded::encode_string(x),
            y: Base64UrlUnpadded::encode_string(y),
        })
    }

    /// Decodes and validates the JWK into a P-256 verifying key.
    ///
    /// # Errors
    /// [`AcmeError::BadPublicKey`] for unsupported `kty`/`crv`, bad
    /// coordinate encoding, or a value that is not a point on the curve.
    pub fn verifying_key(&self) -> Result<VerifyingKey, AcmeError> {
        if self.kty != "EC" {
            return Err(AcmeError::BadPublicKey(format!(
                "unsupported kty {:?} (need \"EC\")",
                self.kty
            )));
        }
        if self.crv != "P-256" {
            return Err(AcmeError::BadPublicKey(format!(
                "unsupported crv {:?} (need \"P-256\")",
                self.crv
            )));
        }
        let x = p256::FieldBytes::from(decode_coordinate("x", &self.x)?);
        let y = p256::FieldBytes::from(decode_coordinate("y", &self.y)?);
        let point = p256::EncodedPoint::from_affine_coordinates(&x, &y, false);
        VerifyingKey::from_encoded_point(&point)
            .map_err(|_| AcmeError::BadPublicKey("coordinates are not a P-256 point".into()))
    }

    /// RFC 7638 thumbprint over the canonical `{"crv","kty","x","y"}` form.
    #[must_use]
    pub fn thumbprint(&self) -> JwkThumbprint {
        // serde_json's default Map is a BTreeMap, so members serialize in
        // lexicographic order — exactly the RFC 7638 canonical order for EC
        // keys (crv, kty, x, y) — with no whitespace.
        let canonical = serde_json::json!({
            "crv": self.crv,
            "kty": self.kty,
            "x": self.x,
            "y": self.y,
        })
        .to_string();
        let digest = Sha256::digest(canonical.as_bytes());
        JwkThumbprint(Base64UrlUnpadded::encode_string(&digest))
    }
}

fn decode_coordinate(name: &str, value: &str) -> Result<[u8; 32], AcmeError> {
    let bytes = Base64UrlUnpadded::decode_vec(value)
        .map_err(|_| AcmeError::BadPublicKey(format!("coordinate {name}: invalid base64url")))?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| AcmeError::BadPublicKey(format!("coordinate {name}: expected 32 bytes")))
}

/// The JWS protected header fields ACME cares about (RFC 8555 §6.2).
#[derive(Deserialize, Clone, Debug)]
pub struct ProtectedHeader {
    /// Signature algorithm. Only `ES256` is accepted.
    pub alg: String,
    /// Anti-replay nonce (required on all authenticated requests).
    #[serde(default)]
    pub nonce: Option<String>,
    /// The exact URL the client believes it is POSTing to (RFC 8555 §6.4).
    #[serde(default)]
    pub url: Option<String>,
    /// Inline public key — used for `new-account` only.
    #[serde(default)]
    pub jwk: Option<Jwk>,
    /// Account URL — used for all other authenticated requests.
    #[serde(default)]
    pub kid: Option<String>,
}

/// Where the verification key comes from (RFC 8555 §6.2: exactly one of
/// `jwk`/`kid`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AccountKeySource<'a> {
    /// Key travels inline (`new-account`).
    Jwk(&'a Jwk),
    /// Key is bound to an existing account URL (`kid`); the verifier must
    /// look the key up in the account store.
    Kid(&'a str),
}

/// Flattened JSON JWS envelope (RFC 7515 §7.2.2).
#[derive(Deserialize)]
struct FlattenedJws {
    protected: String,
    payload: String,
    signature: String,
}

/// A parsed (but not yet verified) ACME JWS request body.
#[derive(Clone, Debug)]
pub struct Jws {
    protected: ProtectedHeader,
    payload: Vec<u8>,
    signature: Vec<u8>,
    /// Original base64url segments, kept for the signing input
    /// `ASCII(protected) || '.' || ASCII(payload)`.
    protected_b64: String,
    payload_b64: String,
}

impl Jws {
    /// Parses a flattened-JSON JWS from raw request-body bytes.
    ///
    /// Total over arbitrary input: returns `Err(Malformed)` rather than
    /// panicking (fuzzed, spec §19.3).
    ///
    /// # Errors
    /// [`AcmeError::Malformed`] for anything that is not a well-formed
    /// envelope: invalid JSON, missing members, bad base64url, or an
    /// undecodable protected header.
    pub fn parse(body: &[u8]) -> Result<Self, AcmeError> {
        let flat: FlattenedJws = serde_json::from_slice(body)
            .map_err(|e| AcmeError::Malformed(format!("invalid JWS request body: {e}")))?;
        let protected_bytes = Base64UrlUnpadded::decode_vec(&flat.protected)
            .map_err(|_| AcmeError::Malformed("protected: invalid base64url".into()))?;
        let protected: ProtectedHeader = serde_json::from_slice(&protected_bytes)
            .map_err(|e| AcmeError::Malformed(format!("invalid protected header: {e}")))?;
        let payload = Base64UrlUnpadded::decode_vec(&flat.payload)
            .map_err(|_| AcmeError::Malformed("payload: invalid base64url".into()))?;
        let signature = Base64UrlUnpadded::decode_vec(&flat.signature)
            .map_err(|_| AcmeError::Malformed("signature: invalid base64url".into()))?;
        Ok(Self {
            protected,
            payload,
            signature,
            protected_b64: flat.protected,
            payload_b64: flat.payload,
        })
    }

    /// The parsed protected header.
    #[must_use]
    pub fn protected(&self) -> &ProtectedHeader {
        &self.protected
    }

    /// The decoded payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Enforces `alg == ES256`.
    ///
    /// # Errors
    /// [`AcmeError::BadSignatureAlgorithm`] for any other value (including
    /// `"none"`).
    pub fn check_alg(&self) -> Result<(), AcmeError> {
        if self.protected.alg == ES256 {
            Ok(())
        } else {
            Err(AcmeError::BadSignatureAlgorithm {
                alg: self.protected.alg.clone(),
            })
        }
    }

    /// Extracts the anti-replay nonce.
    ///
    /// # Errors
    /// [`AcmeError::BadNonce`] when absent or empty (RFC 8555 §6.5: reject
    /// with `badNonce`).
    pub fn nonce(&self) -> Result<Nonce, AcmeError> {
        self.protected
            .nonce
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(Nonce::from_client)
            .ok_or(AcmeError::BadNonce)
    }

    /// Enforces the protected `url` matches the URL actually POSTed to
    /// (RFC 8555 §6.4).
    ///
    /// # Errors
    /// [`AcmeError::Malformed`] when the field is absent;
    /// [`AcmeError::UrlMismatch`] (rendered `unauthorized`) on mismatch.
    pub fn check_url(&self, expected: &str) -> Result<(), AcmeError> {
        match self.protected.url.as_deref() {
            None => Err(AcmeError::Malformed(
                "protected header missing \"url\"".into(),
            )),
            Some(url) if url == expected => Ok(()),
            Some(_) => Err(AcmeError::UrlMismatch),
        }
    }

    /// Determines the account key binding: exactly one of `jwk` / `kid`.
    ///
    /// # Errors
    /// [`AcmeError::Malformed`] when both or neither are present.
    pub fn account_key(&self) -> Result<AccountKeySource<'_>, AcmeError> {
        match (&self.protected.jwk, &self.protected.kid) {
            (Some(jwk), None) => Ok(AccountKeySource::Jwk(jwk)),
            (None, Some(kid)) => Ok(AccountKeySource::Kid(kid)),
            _ => Err(AcmeError::Malformed(
                "protected header must contain exactly one of \"jwk\" or \"kid\"".into(),
            )),
        }
    }

    /// Verifies the ES256 signature over
    /// `ASCII(protected) || '.' || ASCII(payload)`.
    ///
    /// # Errors
    /// [`AcmeError::Malformed`] for a wrong-length signature or a signature
    /// that does not verify under `key`.
    pub fn verify_signature(&self, key: &VerifyingKey) -> Result<(), AcmeError> {
        let signature = Signature::from_slice(&self.signature).map_err(|_| {
            AcmeError::Malformed("signature: expected 64-byte ES256 (r || s)".into())
        })?;
        let signing_input = format!("{}.{}", self.protected_b64, self.payload_b64);
        key.verify(signing_input.as_bytes(), &signature)
            .map_err(|_| AcmeError::Malformed("JWS signature verification failed".into()))
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::SigningKey;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;
    use crate::client::{signed_request_body, ClientBinding};

    fn test_key(seed: u8) -> SigningKey {
        SigningKey::from_slice(&[seed; 32]).expect("valid scalar")
    }

    fn valid_jws(key: &SigningKey) -> Jws {
        let body = signed_request_body(
            key,
            &ClientBinding::Jwk,
            "some-nonce",
            "https://ca.example/acme/new-account",
            &serde_json::json!({"termsOfServiceAgreed": true}),
        )
        .expect("signable");
        Jws::parse(body.as_bytes()).expect("parses")
    }

    #[test]
    fn valid_jws_round_trips_and_verifies() {
        let key = test_key(1);
        let jws = valid_jws(&key);
        assert_eq!(jws.check_alg(), Ok(()));
        assert_eq!(jws.nonce().expect("nonce").as_str(), "some-nonce");
        assert_eq!(jws.check_url("https://ca.example/acme/new-account"), Ok(()));
        let AccountKeySource::Jwk(jwk) = jws.account_key().expect("jwk") else {
            panic!("expected jwk binding");
        };
        let vk = jwk.verifying_key().expect("valid key");
        assert_eq!(jws.verify_signature(&vk), Ok(()));
    }

    #[test]
    fn signature_from_wrong_key_is_rejected() {
        let jws = valid_jws(&test_key(1));
        let other = VerifyingKey::from(&test_key(2));
        assert!(matches!(
            jws.verify_signature(&other),
            Err(AcmeError::Malformed(_))
        ));
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let key = test_key(1);
        let body = signed_request_body(
            &key,
            &ClientBinding::Jwk,
            "n",
            "https://ca.example/x",
            &serde_json::json!({"termsOfServiceAgreed": true}),
        )
        .expect("signable");
        let mut parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
        parsed["payload"] =
            serde_json::Value::String(Base64UrlUnpadded::encode_string(b"{\"evil\":true}"));
        let jws = Jws::parse(parsed.to_string().as_bytes()).expect("parses");
        let vk = VerifyingKey::from(&key);
        assert!(matches!(
            jws.verify_signature(&vk),
            Err(AcmeError::Malformed(_))
        ));
    }

    #[test]
    fn alg_none_is_bad_signature_algorithm() {
        let protected =
            Base64UrlUnpadded::encode_string(br#"{"alg":"none","nonce":"n","url":"u"}"#);
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert_eq!(
            jws.check_alg(),
            Err(AcmeError::BadSignatureAlgorithm { alg: "none".into() })
        );
    }

    #[test]
    fn rs256_is_bad_signature_algorithm() {
        let protected =
            Base64UrlUnpadded::encode_string(br#"{"alg":"RS256","nonce":"n","url":"u"}"#);
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert!(matches!(
            jws.check_alg(),
            Err(AcmeError::BadSignatureAlgorithm { .. })
        ));
    }

    #[test]
    fn missing_nonce_is_bad_nonce() {
        let protected = Base64UrlUnpadded::encode_string(br#"{"alg":"ES256","url":"u"}"#);
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert_eq!(jws.nonce(), Err(AcmeError::BadNonce));
    }

    #[test]
    fn url_mismatch_is_rejected() {
        let jws = valid_jws(&test_key(1));
        assert_eq!(
            jws.check_url("https://ca.example/acme/new-order"),
            Err(AcmeError::UrlMismatch)
        );
    }

    #[test]
    fn missing_url_is_malformed() {
        let protected = Base64UrlUnpadded::encode_string(br#"{"alg":"ES256","nonce":"n"}"#);
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert!(matches!(jws.check_url("x"), Err(AcmeError::Malformed(_))));
    }

    #[test]
    fn jwk_and_kid_together_is_malformed() {
        let key = test_key(1);
        let jwk = Jwk::from_verifying_key(&VerifyingKey::from(&key)).expect("jwk");
        let header = serde_json::json!({
            "alg": "ES256", "nonce": "n", "url": "u",
            "jwk": jwk, "kid": "https://ca.example/acme/acct/1",
        });
        let protected = Base64UrlUnpadded::encode_string(header.to_string().as_bytes());
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert!(matches!(jws.account_key(), Err(AcmeError::Malformed(_))));
    }

    #[test]
    fn neither_jwk_nor_kid_is_malformed() {
        let protected =
            Base64UrlUnpadded::encode_string(br#"{"alg":"ES256","nonce":"n","url":"u"}"#);
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert!(matches!(jws.account_key(), Err(AcmeError::Malformed(_))));
    }

    #[test]
    fn kid_binding_is_surfaced() {
        let protected = Base64UrlUnpadded::encode_string(
            br#"{"alg":"ES256","nonce":"n","url":"u","kid":"https://ca.example/acme/acct/7"}"#,
        );
        let body = serde_json::json!({
            "protected": protected,
            "payload": "",
            "signature": "",
        })
        .to_string();
        let jws = Jws::parse(body.as_bytes()).expect("parses");
        assert_eq!(
            jws.account_key(),
            Ok(AccountKeySource::Kid("https://ca.example/acme/acct/7"))
        );
    }

    #[test]
    fn garbage_body_is_malformed_not_panic() {
        for body in [
            &b""[..],
            b"{}",
            b"[1,2,3]",
            b"{\"protected\":42}",
            b"{\"protected\":\"!!!\",\"payload\":\"\",\"signature\":\"\"}",
            b"\xff\xfe\x00",
        ] {
            assert!(matches!(Jws::parse(body), Err(AcmeError::Malformed(_))));
        }
    }

    #[test]
    fn padded_base64_is_rejected() {
        // RFC 8555 §6.1: base64url with padding stripped; '=' must be refused.
        let body = serde_json::json!({
            "protected": "e30=",
            "payload": "",
            "signature": "",
        })
        .to_string();
        assert!(matches!(
            Jws::parse(body.as_bytes()),
            Err(AcmeError::Malformed(_))
        ));
    }

    #[test]
    fn thumbprint_matches_rfc7638_construction() {
        // Expected value computed independently (python hashlib) over
        // {"crv":"P-256","kty":"EC","x":"abc","y":"xyz"}.
        let jwk = Jwk {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: "abc".into(),
            y: "xyz".into(),
        };
        assert_eq!(
            jwk.thumbprint().as_str(),
            "fICbl_E_7xvtsvuRNR-4mVA2ZX_6Mw49n0FszKczzzI"
        );
    }

    #[test]
    fn thumbprint_is_stable_and_key_specific() {
        let a = Jwk::from_verifying_key(&VerifyingKey::from(&test_key(1))).expect("jwk");
        let b = Jwk::from_verifying_key(&VerifyingKey::from(&test_key(2))).expect("jwk");
        assert_eq!(a.thumbprint(), a.thumbprint());
        assert_ne!(a.thumbprint(), b.thumbprint());
    }

    #[test]
    fn jwk_round_trips_through_verifying_key() {
        let vk = VerifyingKey::from(&test_key(3));
        let jwk = Jwk::from_verifying_key(&vk).expect("jwk");
        assert_eq!(jwk.verifying_key().expect("decodes"), vk);
    }

    #[test]
    fn bad_public_keys_are_rejected() {
        let good = Jwk::from_verifying_key(&VerifyingKey::from(&test_key(1))).expect("jwk");
        let cases = [
            Jwk {
                kty: "RSA".into(),
                ..good.clone()
            },
            Jwk {
                crv: "P-384".into(),
                ..good.clone()
            },
            Jwk {
                x: "!!!".into(),
                ..good.clone()
            },
            Jwk {
                x: "AAAA".into(),
                ..good.clone()
            }, // wrong length
            // Valid encodings that are not a point on the curve:
            Jwk {
                x: Base64UrlUnpadded::encode_string(&[0u8; 32]),
                y: Base64UrlUnpadded::encode_string(&[1u8; 32]),
                ..good.clone()
            },
        ];
        for jwk in cases {
            assert!(matches!(
                jwk.verifying_key(),
                Err(AcmeError::BadPublicKey(_))
            ));
        }
    }

    proptest! {
        /// Spec §19.3 layer 1: the parser is total — no panic on arbitrary
        /// bytes (mirrors the cargo-fuzz target in `fuzz/`).
        #[test]
        fn parse_never_panics_on_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Jws::parse(&data);
        }

        /// Structured variant: a well-formed envelope with arbitrary segment
        /// contents exercises the base64/header paths.
        #[test]
        fn parse_never_panics_on_arbitrary_envelope(
            protected in "[A-Za-z0-9_=!-]{0,64}",
            payload in "[A-Za-z0-9_=!-]{0,64}",
            signature in "[A-Za-z0-9_=!-]{0,64}",
        ) {
            let body = serde_json::json!({
                "protected": protected,
                "payload": payload,
                "signature": signature,
            }).to_string();
            let _ = Jws::parse(body.as_bytes());
        }
    }
}
