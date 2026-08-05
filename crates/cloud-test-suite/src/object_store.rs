//! Shared [`ObjectStore`] conformance suite (spec §9.7).
//!
//! Ported from `cloud-memory`'s original inline `#[cfg(test)]` module (spec
//! §9.6) -- the assertions are unchanged, generalized behind the
//! factory-closure pattern so any backend can run them.

use std::future::Future;

use cloud_types::{CloudError, ObjectStore, PutOptions};

/// Runs the full [`ObjectStore`] conformance suite against instances built by
/// `factory`.
///
/// `factory` is called once per sub-test case (spec §9.7's factory-closure
/// pattern) so cases never share state: each call must return a backend
/// instance isolated from every other call's instance (a fresh in-memory
/// map, a freshly-provisioned bucket/prefix, ...).
///
/// # Panics
///
/// Panics (via `assert!`/`assert_eq!`) on the first behavior that diverges
/// from the contract documented on [`ObjectStore`] -- the same failure mode
/// as any other test.
pub async fn run_object_store_suite<F, Fut, S>(factory: F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore + 'static,
{
    test_put_then_get_round_trips(&factory).await;
    test_get_missing_is_not_found(&factory).await;
    test_if_not_exists_put_rejects_overwrite(&factory).await;
    test_overwrite_mode_replaces_existing_object(&factory).await;
    test_head_reports_matching_size(&factory).await;
    test_head_missing_is_not_found(&factory).await;
    test_list_returns_only_matching_prefix_sorted_by_key(&factory).await;
    test_list_with_no_matches_is_empty_ok(&factory).await;
    test_delete_removes_object(&factory).await;
    test_delete_missing_is_not_found(&factory).await;
}

async fn test_put_then_get_round_trips<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    store
        .put(
            "cts/object-store/round-trip",
            b"leaf",
            PutOptions::default(),
        )
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    let bytes = store
        .get("cts/object-store/round-trip")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"leaf");
}

async fn test_get_missing_is_not_found<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    let result = store.get("cts/object-store/missing").await;
    let Err(err) = result else {
        panic!("get of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_if_not_exists_put_rejects_overwrite<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    store
        .put(
            "cts/object-store/if-not-exists",
            b"leaf",
            PutOptions::if_not_exists(),
        )
        .await
        .unwrap_or_else(|err| panic!("first put should succeed: {err}"));
    let result = store
        .put(
            "cts/object-store/if-not-exists",
            b"other",
            PutOptions::if_not_exists(),
        )
        .await;
    let Err(err) = result else {
        panic!("second if-not-exists put over an occupied key must fail");
    };
    assert!(matches!(err, CloudError::AlreadyExists { .. }), "{err:?}");
    // The append-only invariant: content is unchanged.
    let bytes = store
        .get("cts/object-store/if-not-exists")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"leaf");
}

async fn test_overwrite_mode_replaces_existing_object<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    store
        .put(
            "cts/object-store/overwrite",
            b"first",
            PutOptions::if_not_exists(),
        )
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    store
        .put(
            "cts/object-store/overwrite",
            b"second",
            PutOptions::overwrite(),
        )
        .await
        .unwrap_or_else(|err| panic!("overwrite put should succeed: {err}"));
    let bytes = store
        .get("cts/object-store/overwrite")
        .await
        .unwrap_or_else(|err| panic!("get should succeed: {err}"));
    assert_eq!(bytes, b"second");
}

async fn test_head_reports_matching_size<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    store
        .put("cts/object-store/head", b"leaf!", PutOptions::default())
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    let meta = store
        .head("cts/object-store/head")
        .await
        .unwrap_or_else(|err| panic!("head should succeed: {err}"));
    assert_eq!(meta.size_bytes, 5);
}

async fn test_head_missing_is_not_found<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    let result = store.head("cts/object-store/missing").await;
    let Err(err) = result else {
        panic!("head of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_list_returns_only_matching_prefix_sorted_by_key<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    for key in [
        "cts/object-store/list/entries/0002",
        "cts/object-store/list/entries/0001",
        "cts/object-store/list/tiles/0001",
    ] {
        store
            .put(key, b"x", PutOptions::default())
            .await
            .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    }
    let listed = store
        .list("cts/object-store/list/entries/")
        .await
        .unwrap_or_else(|err| panic!("list should succeed: {err}"));
    let keys: Vec<&str> = listed.iter().map(|info| info.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "cts/object-store/list/entries/0001",
            "cts/object-store/list/entries/0002",
        ]
    );
}

async fn test_list_with_no_matches_is_empty_ok<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    let listed = store
        .list("cts/object-store/list/nothing/")
        .await
        .unwrap_or_else(|err| panic!("list should succeed: {err}"));
    assert_eq!(listed, vec![]);
}

async fn test_delete_removes_object<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    store
        .put("cts/object-store/delete", b"x", PutOptions::default())
        .await
        .unwrap_or_else(|err| panic!("put should succeed: {err}"));
    store
        .delete("cts/object-store/delete")
        .await
        .unwrap_or_else(|err| panic!("delete should succeed: {err}"));
    let result = store.get("cts/object-store/delete").await;
    let Err(err) = result else {
        panic!("get after delete must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}

async fn test_delete_missing_is_not_found<F, Fut, S>(factory: &F)
where
    F: Fn() -> Fut + Sync,
    Fut: Future<Output = S> + Send,
    S: ObjectStore,
{
    let store = factory().await;
    let result = store.delete("cts/object-store/missing").await;
    let Err(err) = result else {
        panic!("delete of a missing key must fail");
    };
    assert!(matches!(err, CloudError::NotFound { .. }), "{err:?}");
}
