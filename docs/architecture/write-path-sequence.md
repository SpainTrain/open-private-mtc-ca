# Write Path Sequence

> **Source of truth:** [`docs/mtc-architecture-spec.md`](../mtc-architecture-spec.md) §11 (Write Path),
> specifically the §11.1 lifecycle and §11.2 critical invariants.
> This diagram renders the nine lifecycle steps: intake → batch builder → tree updater → checkpoint signer → commit → deliver.
>
> **Update this page when:** §11.1 steps are added/removed/reordered, the linearization point moves,
> ordering invariants change (S3-first/DDB-second), or the epoch/lease conditional-write rules in
> §11.2 change. Any PR that changes `CaService::issue_batch` semantics must update this diagram in
> the same PR.

## Issuance lifecycle (spec §11.1 steps 1–9)

```mermaid
sequenceDiagram
    participant Src as ACME endpoint / Adapter
    participant Intake as Entry intake
    participant BB as Batch builder
    participant DDB as DynamoDB (coordination)
    participant S3 as S3 (log content)
    participant TU as Tree updater
    participant HSM as CloudHSM (checkpoint signer)

    Src->>Intake: SubmitEntry(LogEntry)
    Intake->>BB: drain queue (cadence 2-5s or 256 entries)

    Note over BB,DDB: Step 1 - lease check
    BB->>DDB: read primary-region-lease
    DDB-->>BB: lease{region, expires_at, epoch}
    Note right of BB: not primary or expiring soon -> abort (503)

    Note over BB,DDB: Step 3 - allocate indices
    BB->>DDB: UpdateItem counter (ConditionExpression epoch = :epoch)
    DDB-->>BB: [start, end)

    Note over BB,DDB: Step 4 - persist batch state
    BB->>DDB: put batch#id status=pending, epoch

    Note over BB,S3: Step 5 - write entries (parallel, idempotent)
    BB->>S3: PutObject entries/.../NNNNNN.entry (xN)

    Note over TU,S3: Step 6 - tree update
    BB->>TU: entries [start, end)
    TU->>S3: PutObject new tiles (immutable)
    TU-->>BB: new root hash

    Note over BB,HSM: Step 7 - sign checkpoint
    BB->>HSM: sign(tree_size, root_hash, timestamp)
    HSM-->>BB: signature

    rect rgb(235, 220, 220)
        Note over BB,DDB: Step 8 - COMMIT (linearization point)
        BB->>S3: PutObject checkpoints/tree_size.signed (deterministic, idempotent)
        BB->>DDB: TransactWriteItems: latest-checkpoint pointer + batch committed (epoch-conditional)
    end

    Note over BB,Src: Step 9 - assemble and deliver
    BB-->>Src: MTC certificate (TBSCert + MTCProof + CA sig) via ACME finalize / adapter notify
```

## Invariants this diagram encodes (spec §11.2)

- **Step 8 is the linearization point** — nothing is issued until the checkpoint object exists
  and the DDB transaction commits.
- **Counter never decreases** — abandoned indices become permanent gaps filled with `null_entry`.
- **Epoch in every conditional write** (steps 3, 4, 8) — an old primary cannot write after an
  epoch advance (see [`multi-region-topology.md`](multi-region-topology.md)).
- **S3 first, DDB second** — orphan S3 objects from failed transactions are harmless and cleaned
  up by lifecycle policy / orphan-cleanup Lambda (§11.3).
- Step numbering follows §11.1; step 2 (batch assemble) is the `Intake → Batch builder` drain edge.
