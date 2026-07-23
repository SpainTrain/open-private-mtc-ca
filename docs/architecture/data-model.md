# Data Model

> **Source of truth:** [`docs/mtc-architecture-spec.md`](../mtc-architecture-spec.md) §8
> (Data Model: §8.1 S3 layout, §8.2 DynamoDB schema, §8.3 lease semantics, §8.4 in-memory state).
> The first diagram renders the §8.1 S3 object layout; the second renders the §8.2 DynamoDB
> single-table item inventory.
>
> **Update this page when:** the S3 prefix layout changes, DDB item types or key attributes are
> added/removed, or the PK/SK pattern changes. Any PR that changes the storage schema must update
> this page in the same PR.

## S3 layout (spec §8.1)

One bucket per region, identical structure, CRR with Replication Time Control. All objects are
immutable once written (Object Lock, Compliance mode); names are fixed-width zero-padded for
lexicographic ordering.

```mermaid
flowchart LR
    bucket[("s3://mtc-log-region/")]

    cp["checkpoints/"]
    tiles["tiles/"]
    entries["entries/"]
    rev["revocations/"]

    cpobj["0000000000000256.signed (signed checkpoint per committed tree size)"]
    tlevel["level/ (0 = leaves, 1..N = interior)"]
    tileobj["000/000.tile (immutable Merkle tile)"]
    entryobj["000/000/000000.entry (one LogEntry per index)"]
    revobj["0000000000000256.signed (signed revocation list)"]

    bucket --> cp --> cpobj
    bucket --> tiles --> tlevel --> tileobj
    bucket --> entries --> entryobj
    bucket --> rev --> revobj
```

## DynamoDB coordination table (spec §8.2)

Single Global Table `mtc-log-coordination`, replicated to all three regions.
`PK = log#{logId}`; item type is encoded in the `SK` pattern. This is coordination state only —
log content lives exclusively in S3.

```mermaid
erDiagram
    LOG ||--|| COUNTER : "SK counter"
    LOG ||--|| PRIMARY_REGION_LEASE : "SK primary-region-lease"
    LOG ||--|| LATEST_CHECKPOINT : "SK latest-checkpoint"
    LOG ||--|| LATEST_REVOCATION : "SK latest-revocation"
    LOG ||--o{ BATCH : "SK batch#batchId"
    LOG ||--o{ AUDIT : "SK audit#tree_size"

    LOG {
        string partition_key PK "log#logId"
    }
    COUNTER {
        number next_index
        number epoch
    }
    PRIMARY_REGION_LEASE {
        string region
        number expires_at "renewed every 20s, 60s TTL"
        number epoch "incremented on every takeover"
        string holder_id
    }
    LATEST_CHECKPOINT {
        number tree_size
        string s3_key
        number signed_at
        number epoch
    }
    LATEST_REVOCATION {
        number tree_size
        string s3_key
        number signed_at
        number epoch
    }
    BATCH {
        string status "pending or committed or abandoned"
        number start_index
        number end_index
        number leaf_count
        number epoch
        number created_at
        number committed_at
        string source_type "adapter pattern, spec sect. 10"
        string source_id
    }
    AUDIT {
        string proof "self-auditor proof of correct operation"
    }
```

## Reading guide

- **Epoch appears on every mutable item** — every conditional write includes
  `epoch = :currentEpoch`, which is what makes split-brain impossible (spec §8.3, §13.4).
- **S3 is content, DDB is coordination**: proofs and certificates are served entirely from S3
  objects; DDB items only point at them (`s3_key`) and sequence the writers.
- In-memory CA Service state (tile LRU cache, checkpoint cache, intake queue, optimistic counter
  view — spec §8.4) is deliberately not diagrammed as durable state: it is reconstructible from
  S3 + DDB.
