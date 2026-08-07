//! Every [`Storage`] method returns [`StorageError::Unimplemented`], never a
//! panic (ticket `mtc-f35` AC: "unimplemented methods return the typed
//! Unimplemented error ..., never panic").

use std::sync::Arc;

use cloud_backend::{build_backend, BackendConfig, Provider};
use mtc::{
    BatchId, Checkpoint, CheckpointBuilder, EcdsaP256, Epoch, HashOutput, Index, LogEntry, LogId,
    Signed, SignedAt, Tile, TileCoord, TileIndex, TileLevel, TileWidth, TreeSize,
};
use storage::{BatchState, BatchStatus, S3DdbStorage, Storage, StorageConfig, StorageError};

// Non-#[test] helpers in an integration-test file: the allow-expect-in-tests
// clippy.toml exemption does not auto-apply (docs/lint-policy.md deviation
// 1) -- scoped allow with the same justification: test fixture setup, not
// production code.

#[allow(clippy::expect_used)]
async fn storage_over_memory_backend() -> S3DdbStorage {
    let backend = build_backend(BackendConfig {
        provider: Provider::Memory,
    })
    .await
    .expect("the memory provider always builds");
    let config = StorageConfig {
        bucket: "mtc-log-1".to_string(),
        table: "mtc-coordination".to_string(),
        log_id: "log-1".to_string(),
        retention_days: 2555,
    };
    S3DdbStorage::new(Arc::new(backend), config)
}

#[allow(clippy::expect_used)]
fn sample_batch_id() -> BatchId {
    BatchId::new("batch-1").expect("non-empty id")
}

#[allow(clippy::expect_used)]
fn sample_tile() -> Tile {
    let coord = TileCoord::new(
        TileLevel(0),
        TileIndex(0),
        TileWidth::new(1).expect("1..=256"),
    );
    Tile::new(coord, vec![HashOutput([0u8; 32])]).expect("hash count matches width 1")
}

#[allow(clippy::expect_used)]
fn sample_checkpoint() -> Checkpoint<Signed> {
    let scheme = EcdsaP256;
    let (signing, _verifying) = EcdsaP256::generate_keypair();
    CheckpointBuilder::new(LogId::new("log-1").expect("non-empty id"))
        .root_hash(HashOutput([0u8; 32]))
        .tree_size(TreeSize(0))
        .signed_at(SignedAt(0))
        .build()
        .sign(&scheme, &signing)
        .expect("signing a well-formed checkpoint succeeds")
}

fn sample_batch_state() -> BatchState {
    BatchState {
        batch_id: sample_batch_id(),
        start: Index(0),
        end: Index(4),
        status: BatchStatus::Pending,
        epoch: Epoch(0),
    }
}

#[tokio::test]
async fn read_lease_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    assert_eq!(
        storage.read_lease().await,
        Err(StorageError::Unimplemented {
            method: "read_lease"
        }),
    );
}

#[tokio::test]
async fn allocate_indices_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    assert_eq!(
        storage.allocate_indices(4, Epoch(0)).await,
        Err(StorageError::Unimplemented {
            method: "allocate_indices"
        }),
    );
}

#[tokio::test]
async fn persist_batch_state_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    let batch_id = sample_batch_id();
    assert_eq!(
        storage
            .persist_batch_state(
                &batch_id,
                Index(0),
                Index(4),
                BatchStatus::Pending,
                Epoch(0)
            )
            .await,
        Err(StorageError::Unimplemented {
            method: "persist_batch_state"
        }),
    );
}

#[tokio::test]
async fn write_entries_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    let entries = [LogEntry::null()];
    assert_eq!(
        storage.write_entries(Index(0), &entries).await,
        Err(StorageError::Unimplemented {
            method: "write_entries"
        }),
    );
}

#[tokio::test]
async fn write_tiles_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    let tiles = [sample_tile()];
    assert_eq!(
        storage.write_tiles(&tiles).await,
        Err(StorageError::Unimplemented {
            method: "write_tiles"
        }),
    );
}

#[tokio::test]
async fn commit_checkpoint_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    let checkpoint = sample_checkpoint();
    let batch_id = sample_batch_id();
    assert_eq!(
        storage
            .commit_checkpoint(&checkpoint, &batch_id, Epoch(0))
            .await,
        Err(StorageError::Unimplemented {
            method: "commit_checkpoint"
        }),
    );
}

#[tokio::test]
async fn read_latest_checkpoint_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    assert_eq!(
        storage.read_latest_checkpoint().await,
        Err(StorageError::Unimplemented {
            method: "read_latest_checkpoint"
        }),
    );
}

#[tokio::test]
async fn query_pending_batches_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    assert_eq!(
        storage.query_pending_batches().await,
        Err(StorageError::Unimplemented {
            method: "query_pending_batches"
        }),
    );
}

#[tokio::test]
async fn mark_batch_abandoned_is_unimplemented() {
    let storage = storage_over_memory_backend().await;
    let batch = sample_batch_state();
    assert_eq!(
        storage.mark_batch_abandoned(&batch).await,
        Err(StorageError::Unimplemented {
            method: "mark_batch_abandoned"
        }),
    );
}
