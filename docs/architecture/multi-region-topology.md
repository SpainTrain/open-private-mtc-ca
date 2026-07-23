# Multi-Region Active-Passive Topology

> **Source of truth:** [`docs/mtc-architecture-spec.md`](../mtc-architecture-spec.md) §7
> (High-Level Architecture: three-region deployment), §8.3 (lease semantics), and §13
> (Failover and Disaster Recovery).
> The topology diagram renders §7; the failover state diagram renders §8.3 + §13.
>
> **Update this page when:** the region set changes, replication mechanics change (CRR/RTC,
> Global Tables), lease timing constants change (renew interval, TTL), or the failover procedure
> in §13.3 changes. Any PR touching lease/epoch coordination or promotion logic must update this
> page in the same PR.

## Topology (spec §7)

Three regions deployed identically; exactly one holds the primary-region lease at any moment.

```mermaid
flowchart TB
    subgraph control["Control Plane (Route 53)"]
        hc["Health checks per region + DNS read routing + failover orchestration"]
    end

    subgraph use1["us-east-1 (PRIMARY - holds lease)"]
        fargate1["ECS Fargate: CA Service (full write path)"]
        lambda1["Lambda: proof + glue"]
        hsm1["CloudHSM"]
        s31["S3 (primary)"]
        ddb1["DDB Global Table replica"]
    end

    subgraph usw2["us-west-2 (STANDBY)"]
        fargate2["ECS Fargate: idle / lease-aware standby (writes -> 503)"]
        lambda2["Lambda: proof + glue"]
        hsm2["CloudHSM"]
        s32["S3 (replica)"]
        ddb2["DDB Global Table replica"]
    end

    subgraph euw1["eu-west-1 (STANDBY)"]
        fargate3["ECS Fargate: idle / lease-aware standby (writes -> 503)"]
        lambda3["Lambda: proof + glue"]
        hsm3["CloudHSM"]
        s33["S3 (replica)"]
        ddb3["DDB Global Table replica"]
    end

    hc --> use1
    hc --> usw2
    hc --> euw1

    s31 <-- "CRR + RTC" --> s32
    s32 <-- "CRR + RTC" --> s33
    ddb1 <-- "Global Tables replication" --> ddb2
    ddb2 <-- "Global Tables replication" --> ddb3

    fargate2 -. "monitor lease (read-only)" .-> ddb2
    fargate3 -. "monitor lease (read-only)" .-> ddb3
    fargate1 -- "renew lease every 20s (60s TTL)" --> ddb1
```

## Lease and failover state machine (spec §8.3, §13)

```mermaid
stateDiagram-v2
    [*] --> Standby: region deployed
    Standby --> Standby: monitor lease (no writes, 503)
    Standby --> Promoting: mtcctl failover initiate (manual, v1)
    Promoting --> Primary: claim lease + increment epoch (atomic)
    Primary --> Primary: renew lease every 20s
    Primary --> Demoted: lease lost / epoch advanced elsewhere
    Demoted --> Standby: stand down (reads lease, sees higher epoch)

    note right of Promoting
        Idempotent promotion (spec 13.3):
        1. verify latest checkpoint signature
        2. wait for CRR consistency
        3. abandon in-flight batches
        4. claim lease, epoch = epoch + 1
        5. fill index gaps with null_entry
    end note

    note right of Demoted
        Split-brain impossible: every write is
        conditional on epoch = current epoch
        (spec 13.4)
    end note
```

## Reading guide

- **Active-passive**: the write path executes only in the lease-holding region; standbys reject
  writes with 503 (spec §11). Reads are served from every region via Route 53 routing.
- **Lease timing** (spec §8.3): renewed every 20s by the holder, 60s TTL; expiry beyond the safety
  margin makes the lease takeover-eligible.
- **Failover is manual in v1** (spec §13.2): a human runs `mtcctl failover initiate`; RTO 5–15 min,
  RPO zero for committed work (spec §13.5).
