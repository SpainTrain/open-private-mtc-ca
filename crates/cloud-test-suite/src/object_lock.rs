//! Shared [`ObjectLock`] conformance suite (spec §9.7).
//!
//! Ported from `cloud-memory`'s original inline `#[cfg(test)]` module (spec
//! §9.6) -- the assertions are unchanged, generalized behind the
//! factory-closure pattern so any backend can run them.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use clock::Clock;
use cloud_types::{CloudError, ObjectLock, ObjectStore, PutOptions};

/// Runs the full [`ObjectLock`] conformance suite against instances built by
/// `factory`.
///
/// Every case needs both [`ObjectLock`] (retention) and
/// [`ObjectStore`] (delete/put) on the *same* instance, mirroring the
/// coupling every real backend has (one S3 bucket with Object Lock enabled;
/// one struct implementing both traits) -- see `cloud-memory`'s
/// `MemoryObjectStore` / `MemoryObjectLock` alias for the worked example.
///
/// `clock` supplies retention instants (`clock.now() + duration`) instead of
/// reading `SystemTime::now()` directly, per the injected-`Clock` pattern
/// (rule `no-systemtime-now-in-prod`, spec §22.11 -- clippy's
/// `disallowed-methods` lint has no `#[cfg(test)]` exemption for this rule,
/// so it applies even to test-support code like this suite). Pass the same
/// clock the backend itself was constructed with, so "now" agrees on both
/// sides.
///
/// This suite deliberately does not cover retention *expiry* (an object
/// becoming deletable once its window has passed): proving that needs
/// fast-forwarding wall-clock time, which only a fake/injectable-clock
/// backend can do on demand. That is a backend-specific capability, not a
/// cross-backend contract requirement, so it stays as `cloud-memory`'s own
/// test (using `clock::FakeClock::advance`) rather than living here.
///
/// # Panics
///
/// Panics (via `assert!`/`assert_eq!`) on the first behavior that diverges
/// from the contract documented on [`ObjectLock`].
pub async fn run_object_lock_suite<F, Fut, S>(factory: F, clock: Arc<dyn Clock>)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock + 'static,
{
    test_put_with_retention_then_get_retention_round_trips(&factory, clock.as_ref()).await;
    test_put_with_retention_is_create_only(&factory, clock.as_ref()).await;
    test_get_retention_missing_object_is_not_found(&factory).await;
    test_get_retention_on_unlocked_object_is_not_found(&factory).await;
    test_extend_retention_forward_succeeds(&factory, clock.as_ref()).await;
    test_extend_retention_rejects_shortening(&factory, clock.as_ref()).await;
    test_extend_retention_rejects_equal_instant(&factory, clock.as_ref()).await;
    test_extend_retention_missing_object_is_not_found(&factory, clock.as_ref()).await;
    test_delete_during_retention_window_is_rejected(&factory, clock.as_ref()).await;
    test_overwrite_during_retention_window_is_rejected(&factory, clock.as_ref()).await;
    test_if_not_exists_put_over_retained_key_is_already_exists(&factory, clock.as_ref()).await;
}

async fn test_put_with_retention_then_get_retention_round_trips<F, Fut, S>(
    factory: &F,
    clock: &dyn Clock,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/round-trip", b"cp", retain_until)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let got = store
        .get_retention("cts/object-lock/round-trip")
        .await
        .unwrap_or_else(|err| panic!("get_retention should succeed: {err}"));
    assert_eq!(got, retain_until);
    let bytes = store
        .get("cts/object-lock/round-trip")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"cp");
}

async fn test_put_with_retention_is_create_only<F, Fut, S>(factory: &F, clock: &dyn Clock)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/create-only", b"cp", retain_until)
        .await
        .unwrap_or_else(|err| panic!("first put_with_retention should succeed: {err}"));
    let result = store
        .put_with_retention("cts/object-lock/create-only", b"cp2", retain_until)
        .await;
    let Err(err) = result else {
        panic!("second put_with_retention over an occupied key must fail");
    };
    assert!(matches!(err, CloudError::AlreadyExists { .. }), "{err:?}");
}

async fn test_get_retention_missing_object_is_not_found<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let result = store.get_retention("cts/object-lock/missing").await;
    let Err(err) = result else {
        panic!("get_retention of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_get_retention_on_unlocked_object_is_not_found<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    store
        .put("cts/object-lock/unlocked", b"x", PutOptions::default())
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    let result = store.get_retention("cts/object-lock/unlocked").await;
    let Err(err) = result else {
        panic!("get_retention on an object with no retention lock must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_extend_retention_forward_succeeds<F, Fut, S>(factory: &F, clock: &dyn Clock)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let first = clock.now() + Duration::from_hours(1);
    let second = clock.now() + Duration::from_hours(2);
    store
        .put_with_retention("cts/object-lock/extend-forward", b"cp", first)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    store
        .extend_retention("cts/object-lock/extend-forward", second)
        .await
        .unwrap_or_else(|err| panic!("forward extend should succeed: {err}"));
    let got = store
        .get_retention("cts/object-lock/extend-forward")
        .await
        .unwrap_or_else(|err| panic!("get_retention should succeed: {err}"));
    assert_eq!(got, second);
}

async fn test_extend_retention_rejects_shortening<F, Fut, S>(factory: &F, clock: &dyn Clock)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let first = clock.now() + Duration::from_hours(2);
    let shorter = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/extend-shorten", b"cp", first)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let result = store
        .extend_retention("cts/object-lock/extend-shorten", shorter)
        .await;
    let Err(err) = result else {
        panic!("shortening extend_retention must fail");
    };
    assert!(
        matches!(err, CloudError::RetentionViolation { .. }),
        "{err:?}"
    );
    // Unchanged.
    let got = store
        .get_retention("cts/object-lock/extend-shorten")
        .await
        .unwrap_or_else(|err| panic!("get_retention should succeed: {err}"));
    assert_eq!(got, first);
}

async fn test_extend_retention_rejects_equal_instant<F, Fut, S>(factory: &F, clock: &dyn Clock)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/extend-equal", b"cp", retain_until)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let result = store
        .extend_retention("cts/object-lock/extend-equal", retain_until)
        .await;
    let Err(err) = result else {
        panic!("no-op extend_retention must fail");
    };
    assert!(
        matches!(err, CloudError::RetentionViolation { .. }),
        "{err:?}"
    );
}

async fn test_extend_retention_missing_object_is_not_found<F, Fut, S>(
    factory: &F,
    clock: &dyn Clock,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let result = store
        .extend_retention(
            "cts/object-lock/missing",
            clock.now() + Duration::from_hours(1),
        )
        .await;
    let Err(err) = result else {
        panic!("extend_retention of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

// --- Cross-trait: ObjectStore::delete / put must honor ObjectLock retention
// because both traits share one backing store (spec §9.5). ---

async fn test_delete_during_retention_window_is_rejected<F, Fut, S>(factory: &F, clock: &dyn Clock)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/delete-retained", b"cp", retain_until)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let result = store.delete("cts/object-lock/delete-retained").await;
    let Err(err) = result else {
        panic!("delete during an active retention window must fail");
    };
    assert!(
        matches!(err, CloudError::RetentionViolation { .. }),
        "{err:?}"
    );
    // Still there.
    let bytes = store
        .get("cts/object-lock/delete-retained")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"cp");
}

async fn test_overwrite_during_retention_window_is_rejected<F, Fut, S>(
    factory: &F,
    clock: &dyn Clock,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention("cts/object-lock/overwrite-retained", b"cp", retain_until)
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let result = store
        .put(
            "cts/object-lock/overwrite-retained",
            b"tampered",
            PutOptions::overwrite(),
        )
        .await;
    let Err(err) = result else {
        panic!("overwrite during an active retention window must fail");
    };
    assert!(
        matches!(err, CloudError::RetentionViolation { .. }),
        "{err:?}"
    );
    let bytes = store
        .get("cts/object-lock/overwrite-retained")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"cp");
}

async fn test_if_not_exists_put_over_retained_key_is_already_exists<F, Fut, S>(
    factory: &F,
    clock: &dyn Clock,
) where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + ObjectLock,
{
    // AlreadyExists is the primary contract of IfNotExists -- it fires before
    // any retention check, regardless of whether the occupant happens to be
    // retained.
    let store = factory().await;
    let retain_until = clock.now() + Duration::from_hours(1);
    store
        .put_with_retention(
            "cts/object-lock/if-not-exists-retained",
            b"cp",
            retain_until,
        )
        .await
        .unwrap_or_else(|err| panic!("put_with_retention should succeed: {err}"));
    let result = store
        .put(
            "cts/object-lock/if-not-exists-retained",
            b"tampered",
            PutOptions::if_not_exists(),
        )
        .await;
    let Err(err) = result else {
        panic!("if-not-exists put over an occupied (retained) key must fail");
    };
    assert!(matches!(err, CloudError::AlreadyExists { .. }), "{err:?}");
}
