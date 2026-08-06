//! The [`CheckpointSigner`]: write-path step 7 (spec §11.1) end to end — build,
//! HSM-sign, and frame a signed checkpoint object.

use std::sync::Arc;

use clock::tokio::AsyncClock;
use cloud_types::{Hsm, KeyHandle};
use mtc::{CheckpointBuilder, HashOutput, LogId, SignedAt, TreeSize};

use crate::error::CheckpointSignError;
use crate::framing::SignedCheckpointObject;
use crate::retry::RetryPolicy;
use crate::P1363_SIGNATURE_LEN;

/// Builds a checkpoint, signs it on the HSM (ECDSA P-256, spec §14.1), and
/// frames the §8.1 signed-checkpoint object — write-path step 7 (spec §11.1).
///
/// # Dispatch (spec §22.7)
///
/// The HSM is held as `Arc<dyn Hsm>` (a runtime-selected cloud abstraction) and
/// the clock as `Arc<dyn AsyncClock>` (an injected dependency): both are the
/// deliberate `Arc<dyn Trait>` seams, initialized once and shared, never a hot
/// path. Time is read only through the injected clock (rule
/// `no-systemtime-now-in-prod`); this type never touches `SystemTime::now` or
/// `tokio::time`.
///
/// The signer holds no key material — the private key lives inside the HSM and
/// is named by a [`KeyHandle`] passed per call (spec §14).
pub struct CheckpointSigner {
    hsm: Arc<dyn Hsm>,
    clock: Arc<dyn AsyncClock>,
    retry: RetryPolicy,
}

impl CheckpointSigner {
    /// Creates a signer over the HSM signing seam and the injected clock, using
    /// `retry` for transient-failure backoff (spec §11.3 row 7).
    #[must_use]
    pub const fn new(hsm: Arc<dyn Hsm>, clock: Arc<dyn AsyncClock>, retry: RetryPolicy) -> Self {
        Self { hsm, clock, retry }
    }

    /// Builds the checkpoint over `(log_id, tree_size, root_hash, signed_at)`,
    /// signs its canonical `MTCSubtreeSignatureInput` on the HSM, and frames the
    /// §8.1 object.
    ///
    /// The four values flow through mtc's typestate
    /// [`CheckpointBuilder`](mtc::CheckpointBuilder) (spec §22.2): all four are
    /// required, so an incomplete checkpoint cannot be constructed — omitting
    /// one is a compile error, here as in mtc. `signed_at` is supplied by the
    /// caller from the injected clock (it is unauthenticated metadata — draft
    /// §5.4.1 does not sign it).
    ///
    /// Transient HSM failures are retried with backoff; a terminal failure or an
    /// exhausted retry budget surfaces [`CheckpointSignError::HsmSigningFailed`].
    ///
    /// # Errors
    ///
    /// - [`CheckpointSignError::Input`] — the log's `TrustAnchorID` is not
    ///   1..=255 bytes, so the signature input cannot be assembled.
    /// - [`CheckpointSignError::HsmSigningFailed`] — persistent HSM failure
    ///   (distinct, alert-worthy; spec §11.3 row 7).
    /// - [`CheckpointSignError::MalformedSignature`] — the HSM returned a
    ///   non-64-byte signature (wrong-curve key or broken token).
    pub async fn sign_checkpoint(
        &self,
        key_handle: &KeyHandle,
        log_id: LogId,
        tree_size: TreeSize,
        root_hash: HashOutput,
        signed_at: SignedAt,
    ) -> Result<SignedCheckpointObject, CheckpointSignError> {
        let checkpoint = CheckpointBuilder::new(log_id)
            .root_hash(root_hash)
            .tree_size(tree_size)
            .signed_at(signed_at)
            .build();

        // Crown jewel: the exact domain-separated bytes the HSM signs. Assembled
        // by mtc (the `mtc-subtree/v1` label + canonicalization, draft §5.4.1);
        // this crate never re-implements it.
        let signing_input = checkpoint.signature_input()?;
        let signature = self.sign_with_retry(key_handle, &signing_input).await?;

        // The Hsm::sign contract is a fixed 64-byte P1363 r‖s (ADR-0003);
        // anything else is a wrong-curve key or a broken token — alert-worthy,
        // and retrying cannot fix an encoding.
        if signature.len() != P1363_SIGNATURE_LEN {
            return Err(CheckpointSignError::MalformedSignature {
                expected: P1363_SIGNATURE_LEN,
                actual: signature.len(),
            });
        }

        SignedCheckpointObject::frame(&checkpoint, &signature)
    }

    /// Signs `input` on the HSM, retrying only *transient*
    /// (`CloudError::Transport { retryable: true }`) failures with exponential
    /// backoff timed by the injected clock.
    async fn sign_with_retry(
        &self,
        key_handle: &KeyHandle,
        input: &[u8],
    ) -> Result<Vec<u8>, CheckpointSignError> {
        let mut retries: u32 = 0;
        loop {
            match self.hsm.sign(key_handle, input).await {
                Ok(signature) => return Ok(signature),
                Err(error) => {
                    let attempts = retries.saturating_add(1);
                    if error.is_retryable() && retries < self.retry.max_retries {
                        let backoff = self.retry.backoff_for(retries);
                        retries += 1;
                        // Wait via the injected clock — never SystemTime/tokio
                        // directly (rule no-systemtime-now-in-prod). Under a
                        // FakeClock this completes the instant time is advanced.
                        self.clock.sleep(backoff).await;
                        continue;
                    }
                    return Err(CheckpointSignError::HsmSigningFailed {
                        attempts,
                        source: error,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    use async_trait::async_trait;
    use clock::tokio::AsyncClock;
    use clock::{Clock, FakeClock};
    use cloud_memory::MemoryHsm;
    use cloud_types::{CloudError, Hsm, KeyHandle, KeySpec, PublicKey};
    use mtc::{
        Checkpoint, CheckpointBuilder, EcdsaP256, HashOutput, LogId, SignatureAlgorithm, Signed,
        SignedAt, TreeSize, VerifyingKey, SUBTREE_SIGNATURE_LABEL,
    };
    use pretty_assertions::assert_eq;

    use super::CheckpointSigner;
    use crate::{CheckpointSignError, RetryPolicy};

    /// An [`Hsm`] wrapper that fails the first `fail_first` sign calls with a
    /// fixed error, then delegates to `inner`. Counts every sign call so a test
    /// can assert exactly how many attempts the signer made.
    struct CountingHsm {
        inner: Arc<dyn Hsm>,
        fail_first: u32,
        error: CloudError,
        calls: AtomicU32,
    }

    impl CountingHsm {
        fn new(inner: Arc<dyn Hsm>, fail_first: u32, error: CloudError) -> Self {
            Self {
                inner,
                fail_first,
                error,
                calls: AtomicU32::new(0),
            }
        }

        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Hsm for CountingHsm {
        async fn sign(&self, key_handle: &KeyHandle, data: &[u8]) -> Result<Vec<u8>, CloudError> {
            let prior = self.calls.fetch_add(1, Ordering::SeqCst);
            if prior < self.fail_first {
                return Err(self.error.clone());
            }
            self.inner.sign(key_handle, data).await
        }

        async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, CloudError> {
            self.inner.get_public_key(key_handle).await
        }

        async fn generate_key(&self, spec: KeySpec) -> Result<KeyHandle, CloudError> {
            self.inner.generate_key(spec).await
        }

        fn is_fips_validated(&self) -> bool {
            self.inner.is_fips_validated()
        }
    }

    /// An [`Hsm`] that always returns a fixed byte string as the signature —
    /// used to exercise the wrong-length guard.
    struct ConstSigHsm {
        signature: Vec<u8>,
    }

    #[async_trait]
    impl Hsm for ConstSigHsm {
        async fn sign(&self, _key_handle: &KeyHandle, _data: &[u8]) -> Result<Vec<u8>, CloudError> {
            Ok(self.signature.clone())
        }

        async fn get_public_key(&self, _key_handle: &KeyHandle) -> Result<PublicKey, CloudError> {
            Err(CloudError::NotFound {
                key: "n/a".to_string(),
            })
        }

        async fn generate_key(&self, _spec: KeySpec) -> Result<KeyHandle, CloudError> {
            Err(CloudError::Transport {
                retryable: false,
                reason: "n/a".to_string(),
            })
        }

        fn is_fips_validated(&self) -> bool {
            false
        }
    }

    fn frozen_inputs() -> (LogId, TreeSize, HashOutput, SignedAt) {
        (
            LogId::new("ca").unwrap(),
            TreeSize(2),
            HashOutput([0x11; 32]),
            SignedAt(1_700_000_000),
        )
    }

    #[test]
    fn signing_input_carries_the_frozen_domain_label_and_bytes() {
        // ADR-0005 / draft §5.4.1: the checkpoint signs an
        // MTCSubtreeSignatureInput that begins with the 16-byte label
        // `mtc-subtree/v1\n\0`. Re-assert the crown-jewel bytes from this crate
        // (defence in depth) for a frozen input; `signed_at` is deliberately
        // absent from the signed bytes.
        let cp = CheckpointBuilder::new(LogId::new("ca").unwrap())
            .root_hash(HashOutput([0x11; 32]))
            .tree_size(TreeSize(2))
            .signed_at(SignedAt(1_700_000_000))
            .build();
        let input = cp.signature_input().unwrap();

        assert_eq!(&input[..16], &SUBTREE_SIGNATURE_LABEL);
        assert_eq!(
            &input[..16],
            &[
                0x6d, 0x74, 0x63, 0x2d, 0x73, 0x75, 0x62, 0x74, 0x72, 0x65, 0x65, 0x2f, 0x76, 0x31,
                0x0a, 0x00,
            ],
        );

        let mut expected = Vec::new();
        expected.extend_from_slice(b"mtc-subtree/v1\n\0");
        expected.extend_from_slice(&[0x02, b'c', b'a']); // cosigner_id
        expected.extend_from_slice(&[0x02, b'c', b'a']); // log_id
        expected.extend_from_slice(&0u64.to_be_bytes()); // start = 0
        expected.extend_from_slice(&2u64.to_be_bytes()); // end = tree_size
        expected.extend_from_slice(&[0x11u8; 32]); // root hash
        assert_eq!(input, expected);
        assert_eq!(input.len(), 70);
    }

    #[tokio::test]
    async fn signs_frames_and_the_object_parses_and_verifies() {
        // Happy path end to end over a real (in-memory) ECDSA P-256 HSM: build +
        // sign + frame, then prove the object parses via mtc's read path and its
        // signature verifies under the exported HSM public key.
        let hsm = Arc::new(MemoryHsm::new());
        let handle = hsm.generate_key(KeySpec::EcdsaP256).await.unwrap();
        let public_key = hsm.get_public_key(&handle).await.unwrap();

        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let signer = CheckpointSigner::new(
            Arc::clone(&hsm) as Arc<dyn Hsm>,
            clock,
            RetryPolicy::default(),
        );

        let (log_id, tree_size, root_hash, signed_at) = frozen_inputs();
        let object = signer
            .sign_checkpoint(&handle, log_id, tree_size, root_hash, signed_at)
            .await
            .expect("sign");

        // Addressed by tree size (ADR-0003 B.1/B.2), 16-digit zero-padded.
        assert_eq!(object.key(), "checkpoints/0000000000000002.signed");

        let parsed = Checkpoint::<Signed>::parse_tls_presentation(object.bytes())
            .expect("mtc parses our framed object");
        assert_eq!(parsed.log_id().as_str(), "ca");
        assert_eq!(parsed.tree_size(), tree_size);
        assert_eq!(parsed.root_hash(), &root_hash);
        assert_eq!(parsed.signed_at(), signed_at);
        assert_eq!(parsed.signature().len(), 64);

        let verifying_key = VerifyingKey::from_spki_der(
            SignatureAlgorithm::EcdsaP256Sha256,
            public_key.spki_der().to_vec(),
        );
        parsed
            .verify(&EcdsaP256, &verifying_key)
            .expect("HSM signature verifies against the exported public key");

        // Fixed point of mtc's serializer ⇒ byte-identical to the read path.
        assert_eq!(parsed.serialize_tls_presentation().unwrap(), object.bytes());
    }

    #[tokio::test]
    async fn transient_failure_is_retried_with_clock_mediated_backoff() {
        // One transient failure, then success. A 10-minute base backoff makes
        // the assertion sharp: the retry can only proceed once the *injected*
        // clock is advanced — a wall-clock implementation would hang for
        // minutes instead of completing the instant we advance the FakeClock.
        let inner = Arc::new(MemoryHsm::new());
        let handle = inner.generate_key(KeySpec::EcdsaP256).await.unwrap();
        let hsm = Arc::new(CountingHsm::new(
            Arc::clone(&inner) as Arc<dyn Hsm>,
            1,
            CloudError::Transport {
                retryable: true,
                reason: "throttled".to_string(),
            },
        ));
        let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
        let policy = RetryPolicy {
            max_retries: 5,
            base_backoff: Duration::from_mins(10),
            max_backoff: Duration::from_hours(1),
        };
        let signer = CheckpointSigner::new(
            Arc::clone(&hsm) as Arc<dyn Hsm>,
            Arc::clone(&clock) as Arc<dyn AsyncClock>,
            policy,
        );

        let (log_id, tree_size, root_hash, signed_at) = frozen_inputs();
        let task = tokio::spawn(async move {
            signer
                .sign_checkpoint(&handle, log_id, tree_size, root_hash, signed_at)
                .await
        });

        // Let the first (failing) attempt run and the signer park on the
        // injected-clock backoff.
        while hsm.calls() < 1 {
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "the retry backoff must wait on the injected clock, not busy-wait"
        );
        assert_eq!(clock.now(), UNIX_EPOCH, "no wall-clock time was consumed");

        // Advance past the backoff; the parked sleep wakes and the retry runs.
        clock.advance(Duration::from_mins(10));
        let object = task
            .await
            .expect("signer task joins")
            .expect("the retry succeeds");

        assert_eq!(hsm.calls(), 2, "one transient failure + one success");
        assert_eq!(object.key(), "checkpoints/0000000000000002.signed");
    }

    #[tokio::test]
    async fn persistent_transient_failure_surfaces_the_distinct_alert_error() {
        // Every attempt fails transiently: the signer exhausts its retry budget
        // and surfaces the distinct, alert-worthy HsmSigningFailed (spec §11.3
        // row 7 "alert if persistent").
        let hsm = Arc::new(CountingHsm::new(
            Arc::new(MemoryHsm::new()) as Arc<dyn Hsm>,
            u32::MAX,
            CloudError::Transport {
                retryable: true,
                reason: "HSM offline".to_string(),
            },
        ));
        let clock = Arc::new(FakeClock::new(UNIX_EPOCH));
        let policy = RetryPolicy {
            max_retries: 3,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_mins(1),
        };
        let signer = CheckpointSigner::new(
            Arc::clone(&hsm) as Arc<dyn Hsm>,
            Arc::clone(&clock) as Arc<dyn AsyncClock>,
            policy,
        );

        let (log_id, tree_size, root_hash, signed_at) = frozen_inputs();
        let handle = KeyHandle::new("checkpoint-key");
        let task = tokio::spawn(async move {
            signer
                .sign_checkpoint(&handle, log_id, tree_size, root_hash, signed_at)
                .await
        });

        // Drive time forward until the signer gives up; advancing generously
        // each turn clears whatever backoff it is parked on.
        while !task.is_finished() {
            tokio::task::yield_now().await;
            clock.advance(Duration::from_mins(2));
        }

        let error = task
            .await
            .expect("signer task joins")
            .expect_err("persistent failure must error");
        match error {
            CheckpointSignError::HsmSigningFailed { attempts, source } => {
                assert_eq!(attempts, 4, "one initial attempt + three retries");
                assert!(matches!(
                    source,
                    CloudError::Transport {
                        retryable: true,
                        ..
                    }
                ));
            }
            other => panic!("expected HsmSigningFailed, got {other:?}"),
        }
        assert_eq!(hsm.calls(), 4);
    }

    #[tokio::test]
    async fn terminal_failure_is_not_retried() {
        // A non-retryable error (bad key handle) surfaces immediately as the
        // distinct error, with no retry and no backoff.
        let hsm = Arc::new(CountingHsm::new(
            Arc::new(MemoryHsm::new()) as Arc<dyn Hsm>,
            u32::MAX,
            CloudError::NotFound {
                key: "checkpoint-key".to_string(),
            },
        ));
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let signer = CheckpointSigner::new(
            Arc::clone(&hsm) as Arc<dyn Hsm>,
            clock,
            RetryPolicy::default(),
        );

        let (log_id, tree_size, root_hash, signed_at) = frozen_inputs();
        let error = signer
            .sign_checkpoint(
                &KeyHandle::new("checkpoint-key"),
                log_id,
                tree_size,
                root_hash,
                signed_at,
            )
            .await
            .expect_err("terminal failure must error");

        match error {
            CheckpointSignError::HsmSigningFailed { attempts, source } => {
                assert_eq!(attempts, 1, "a terminal error is not retried");
                assert!(matches!(source, CloudError::NotFound { .. }));
            }
            other => panic!("expected HsmSigningFailed, got {other:?}"),
        }
        assert_eq!(hsm.calls(), 1);
    }

    #[tokio::test]
    async fn wrong_length_hsm_signature_is_a_distinct_malformed_error() {
        let hsm = Arc::new(ConstSigHsm {
            signature: vec![0u8; 63],
        });
        let clock: Arc<dyn AsyncClock> = Arc::new(FakeClock::default());
        let signer = CheckpointSigner::new(hsm as Arc<dyn Hsm>, clock, RetryPolicy::default());

        let (log_id, tree_size, root_hash, signed_at) = frozen_inputs();
        let error = signer
            .sign_checkpoint(
                &KeyHandle::new("checkpoint-key"),
                log_id,
                tree_size,
                root_hash,
                signed_at,
            )
            .await
            .expect_err("a non-64-byte signature must error");

        assert_eq!(
            error,
            CheckpointSignError::MalformedSignature {
                expected: 64,
                actual: 63,
            }
        );
    }
}
