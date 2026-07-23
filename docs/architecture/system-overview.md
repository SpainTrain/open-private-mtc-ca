# System Overview

> **Source of truth:** [`docs/mtc-architecture-spec.md`](../mtc-architecture-spec.md) §7 (High-Level Architecture).
> This diagram renders the §7 component inventory and data flow for a single (primary) region plus the control plane.
>
> **Update this page when:** components are added/removed/renamed in §7, a component moves between
> compute platforms (Fargate vs Lambda), or a storage/coordination dependency changes. Any PR that
> changes §7's component table must update this diagram in the same PR.

## Components and data flow

```mermaid
flowchart TB
    subgraph control["Control Plane (Route 53 + ALB)"]
        r53["Route 53 health checks + DNS routing"]
    end

    client["Clients: ACME subscribers / external CAs / admins"]

    subgraph primary["Primary region (ECS Fargate: CA Service)"]
        acme["ACME / API endpoint"]
        adapter["Adapter API (external CA bridge, spec sect. 10)"]
        admin["Admin UI / Admin API"]
        intake["Entry intake queue (in-memory)"]
        batch["Batch builder"]
        tree["Tree updater"]
        checkpointer["Checkpointer"]
        lease["Lease renewer"]
    end

    subgraph lambdas["Lambda (per region)"]
        proof["proof-server: inclusion proofs + cert downloads"]
        revocation["revocation-processor"]
        pruning["pruning-worker (Step Functions)"]
        auditor["self-auditor"]
        orphan["orphan-cleanup"]
    end

    hsm["CloudHSM (checkpoint signing keys)"]

    subgraph storage["Storage"]
        s3["S3: tiles, entries, checkpoints, revocations (immutable, CRR)"]
        ddb["DynamoDB Global Table: mtc-log-coordination"]
    end

    rp["Relying parties (verification)"]

    client --> r53
    r53 --> acme
    r53 --> adapter
    r53 --> admin
    r53 --> proof

    acme --> intake
    adapter --> intake
    intake --> batch
    batch --> tree
    tree --> checkpointer
    checkpointer --> hsm

    lease --> ddb
    batch --> ddb
    checkpointer --> ddb
    batch --> s3
    tree --> s3
    checkpointer --> s3

    proof --> s3
    revocation --> s3
    revocation --> ddb
    pruning --> s3
    auditor --> s3
    auditor --> ddb
    orphan --> s3

    rp --> proof
```

## Reading guide

- The **write path** flows top-to-bottom inside the primary region: intake → batch builder →
  tree updater → checkpointer (detailed step-by-step in
  [`write-path-sequence.md`](write-path-sequence.md), spec §11).
- Only the **primary** region runs the write path; standby regions run the same deployment in
  lease-aware idle (see [`multi-region-topology.md`](multi-region-topology.md), spec §7/§13).
- **Lambdas** serve the read path and event glue; they never hold the lease and never write
  log content (spec §5, §12).
- **S3** holds all immutable log content; **DynamoDB** holds only coordination state
  (counter, lease, pointers, batch status — see [`data-model.md`](data-model.md), spec §8).
