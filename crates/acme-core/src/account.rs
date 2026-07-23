//! ACME accounts (RFC 8555 §7.1.2, §7.3), keyed by JWK thumbprint
//! (RFC 7638).
//!
//! An ACME account *is* its key: the store is indexed by the key's
//! thumbprint, so re-registering an existing key returns the existing
//! account (RFC 8555 §7.3 — 200 with the current account instead of 201).

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::jws::{Jwk, JwkThumbprint};

/// Opaque account identifier (newtype per `use-newtypes`); appears in
/// account URLs (`.../acme/acct/{id}`) used as JWS `kid` values.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct AccountId(pub u64);

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Account status (RFC 8555 §7.1.2). Deactivation/revocation transitions are
/// out of scope for this ticket, so only `valid` is constructed today.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus {
    /// The account may issue requests.
    Valid,
}

/// A registered ACME account.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Account {
    /// Server-assigned identifier.
    pub id: AccountId,
    /// The account public key (the JWK from `new-account`).
    pub key: Jwk,
    /// RFC 7638 thumbprint of `key` — the store's primary key.
    pub thumbprint: JwkThumbprint,
    /// Current status.
    pub status: AccountStatus,
    /// Contact URLs supplied at registration.
    pub contact: Vec<String>,
    /// Whether the client agreed to the terms of service.
    pub terms_of_service_agreed: bool,
}

/// `new-account` request payload (RFC 8555 §7.3). Unknown fields are
/// ignored; External Account Binding is out of scope (admin-surface epic).
#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewAccountRequest {
    /// Contact URLs (e.g. `mailto:`).
    pub contact: Vec<String>,
    /// Terms-of-service agreement flag.
    pub terms_of_service_agreed: bool,
    /// If set, never create: return the existing account or
    /// `accountDoesNotExist`.
    pub only_return_existing: bool,
}

/// In-memory account store keyed by JWK thumbprint.
///
/// Durable persistence is a later ticket; the store lives behind a lock in
/// [`crate::routes::AcmeState`].
#[derive(Default, Debug)]
pub struct AccountStore {
    next_id: u64,
    by_thumbprint: HashMap<JwkThumbprint, Account>,
    id_index: HashMap<AccountId, JwkThumbprint>,
}

impl AccountStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            by_thumbprint: HashMap::new(),
            id_index: HashMap::new(),
        }
    }

    /// Looks up an account by its key's thumbprint.
    #[must_use]
    pub fn find_by_thumbprint(&self, thumbprint: &JwkThumbprint) -> Option<&Account> {
        self.by_thumbprint.get(thumbprint)
    }

    /// Looks up an account by id (the `kid` binding path for later
    /// authenticated endpoints).
    #[must_use]
    pub fn find_by_id(&self, id: AccountId) -> Option<&Account> {
        self.id_index
            .get(&id)
            .and_then(|tp| self.by_thumbprint.get(tp))
    }

    /// Returns the account registered under `key`, creating it if absent.
    ///
    /// The boolean is `true` when a new account was created — the handler
    /// maps this to 201 vs. 200. Re-registration never mutates the stored
    /// account (RFC 8555 §7.3: the server returns the existing account and
    /// ignores the rest of the request).
    pub fn get_or_create(&mut self, key: Jwk, request: &NewAccountRequest) -> (Account, bool) {
        let thumbprint = key.thumbprint();
        if let Some(existing) = self.by_thumbprint.get(&thumbprint) {
            return (existing.clone(), false);
        }
        let id = AccountId(self.next_id);
        self.next_id += 1;
        let account = Account {
            id,
            key,
            thumbprint: thumbprint.clone(),
            status: AccountStatus::Valid,
            contact: request.contact.clone(),
            terms_of_service_agreed: request.terms_of_service_agreed,
        };
        self.id_index.insert(id, thumbprint.clone());
        self.by_thumbprint.insert(thumbprint, account.clone());
        (account, true)
    }

    /// Number of registered accounts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_thumbprint.len()
    }

    /// Whether the store has no accounts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_thumbprint.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::{SigningKey, VerifyingKey};
    use pretty_assertions::assert_eq;

    use super::*;

    fn jwk(seed: u8) -> Jwk {
        let key = SigningKey::from_slice(&[seed; 32]).expect("valid scalar");
        Jwk::from_verifying_key(&VerifyingKey::from(&key)).expect("jwk")
    }

    #[test]
    fn creates_then_returns_existing_for_same_key() {
        let mut store = AccountStore::new();
        let request = NewAccountRequest {
            contact: vec!["mailto:admin@example.com".into()],
            terms_of_service_agreed: true,
            only_return_existing: false,
        };
        let (created, was_created) = store.get_or_create(jwk(1), &request);
        assert!(was_created);

        // Re-registering the same key: same account, nothing mutated, even
        // with a different request payload.
        let (existing, was_created) = store.get_or_create(jwk(1), &NewAccountRequest::default());
        assert!(!was_created);
        assert_eq!(existing, created);
        assert_eq!(
            existing.contact,
            vec!["mailto:admin@example.com".to_owned()]
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn different_keys_get_distinct_accounts() {
        let mut store = AccountStore::new();
        let (a, _) = store.get_or_create(jwk(1), &NewAccountRequest::default());
        let (b, _) = store.get_or_create(jwk(2), &NewAccountRequest::default());
        assert_ne!(a.id, b.id);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn lookup_by_thumbprint_and_id_agree() {
        let mut store = AccountStore::new();
        let (account, _) = store.get_or_create(jwk(1), &NewAccountRequest::default());
        assert_eq!(
            store.find_by_thumbprint(&account.thumbprint),
            Some(&account)
        );
        assert_eq!(store.find_by_id(account.id), Some(&account));
        assert_eq!(store.find_by_id(AccountId(999)), None);
    }

    #[test]
    fn empty_store_finds_nothing() {
        let store = AccountStore::new();
        assert!(store.is_empty());
        assert_eq!(store.find_by_thumbprint(&jwk(1).thumbprint()), None);
    }

    #[test]
    fn new_account_request_parses_rfc_field_names() {
        let request: NewAccountRequest = serde_json::from_str(
            r#"{"contact":["mailto:a@b.c"],"termsOfServiceAgreed":true,"onlyReturnExisting":true,"unknownField":1}"#,
        )
        .expect("parses");
        assert!(request.terms_of_service_agreed);
        assert!(request.only_return_existing);
        assert_eq!(request.contact, vec!["mailto:a@b.c".to_owned()]);
    }
}
