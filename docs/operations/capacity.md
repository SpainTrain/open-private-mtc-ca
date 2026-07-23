# Capacity Planning Model

This document expands [§21.1 of the architecture spec](../mtc-architecture-spec.md#211-capacity-planning-rough)
into a parameterized capacity model for the whole system. It derives every §21.1
figure from first principles, examines binding constraints and headroom at 1x /
10x / 100x of the baseline load, and states the scaling levers for each
bottleneck.

**Scope.** This is a static planning model. It is distinct from the live,
forward-looking capacity-prediction metrics owned by observability
([§20.4](../mtc-architecture-spec.md#204-capacity-prediction)). Calculator
tooling that mechanically re-derives these numbers is a separate ticket
(`ops-capacity-calculator`); once it lands, its tests re-derive every numeric
example in this document.

**Cost stance.** All costs herein are modeled only. Per the
[§1 non-goals](../mtc-architecture-spec.md#1-goals-and-non-goals), this project
incurs no real AWS spend; development runs entirely on the zero-cost local
simulation environment (LocalStack + SoftHSM, §18).

---

## 1. Model parameters

All parameters come from the spec. No load figures are assumed beyond these.

| Symbol | Meaning | Value | Source |
| --- | --- | --- | --- |
| `R` | Issuance rate (baseline) | 10,000 certs/hour | [§21.1](../mtc-architecture-spec.md#211-capacity-planning-rough) |
| `B` | Batch size (max entries per batch) | 256 | [§11.1](../mtc-architecture-spec.md#111-lifecycle) step 2; [§21.1](../mtc-architecture-spec.md#211-capacity-planning-rough) |
| `I` | Batch cadence (time trigger) | 2–5 s | [§11.1](../mtc-architecture-spec.md#111-lifecycle) step 2 |
| `W` | Tile width (leaves per tile, per tlog-tiles) | 256 | [§2](../mtc-architecture-spec.md#2-background-what-is-mtc) key-concepts table |
| `H` | Hash size (SHA-256) | 32 bytes | [§22.7](../mtc-architecture-spec.md#227-static-vs-dynamic-dispatch-a-deliberate-boundary) (tree updater monomorphizes to SHA-256) |
| `T` | Retention window (default) | 7 years | [§15.1](../mtc-architecture-spec.md#151-model) |
| — | HSM signing latency target | < 100 ms p99 | [§14.3](../mtc-architecture-spec.md#143-performance-target) |
| — | Issuance latency SLO | < 10 s p99 | [§20.1](../mtc-architecture-spec.md#201-service-health) SLO table |

Scale points examined below: **1x = 10,000**, **10x = 100,000**,
**100x = 1,000,000** certs/hour (multiples of the §21.1 baseline; the spec
states no other load assumptions).

---

## 2. First-principles derivation of the §21.1 rates

§21.1 models the *size-bound* regime: every batch fills to `B = 256` entries
before it is emitted. (Section 3 covers the other regime.) One batch commit
drives exactly one pass of the write path
([§11.1](../mtc-architecture-spec.md#111-lifecycle)):

- one HSM checkpoint signing (step 7),
- one DynamoDB `TransactWriteItems` commit transaction (step 8),
- one S3 `PutObject` per entry (step 5),
- one S3 write per affected tile (step 6),
- one S3 checkpoint object write (step 8).

### Batches per hour

```text
batches/hour = R / B = 10,000 / 256 = 39.06  ≈ 40        (§21.1: ~40)
```

### HSM signings per hour

One checkpoint signing per batch commit:

```text
signings/hour = batches/hour = 39.06  ≈ 40               (§21.1: ~40)
```

### DynamoDB commit transactions per hour

One `TransactWriteItems` (update `latest-checkpoint` pointer + mark batch
committed, [§11.1](../mtc-architecture-spec.md#111-lifecycle) step 8) per batch:

```text
commit tx/hour = batches/hour = 39.06  ≈ 40              (§21.1: ~40)
```

(Each batch also performs one `UpdateItem` on the shared `counter` item at
step 3 — same per-batch rate, relevant to the hot-key analysis in §5.2.)

### S3 entry writes per hour

One `PutObject` per entry (step 5), independent of batching:

```text
entry writes/hour = R = 10,000                           (§21.1: ~10K)
```

### S3 tile writes per hour

Tiles are fixed 256-leaf chunks (`W = 256`), stacked in levels: a level-1 tile
covers 256 level-0 tiles (65,536 leaves), a level-2 tile covers 16.78M leaves,
a level-3 tile covers ~4.29B leaves. A tree of `N` leaves has
`ceil(log₂₅₆ N)` tile levels — 4 levels for any tree between 16.8M and 4.29B
leaves (the steady-state range for this model, see §6).

Per full-batch commit (`B = W = 256`), the tree updater (step 6) writes:

- 1 completed level-0 tile (a 256-entry batch spans exactly one tile;
  2 level-0 tiles when the batch straddles a tile boundary),
- the updated rightmost (partial) tile at each higher level: levels 1, 2, 3.

```text
tile writes/commit ≈ 4–5
tile writes/hour   ≈ 5 × 39.06 = 195.3  ≈ 200            (§21.1: ~200)
```

§21.1's "~200" corresponds to ~5 tile writes per commit at ~40 commits/hour.

### Checkpoint writes per hour (not listed in §21.1)

One checkpoint object per commit (step 8): ~40/hour, same as batches/hour.

**Result: every §21.1 figure is reproduced.**

| Quantity | Derived | §21.1 states |
| --- | --- | --- |
| Batches/hour | 39.06 | ~40 |
| HSM signings/hour | 39.06 | ~40 |
| DDB commit transactions/hour | 39.06 | ~40 |
| S3 entry writes/hour | 10,000 | ~10K |
| S3 tile writes/hour | 195.3 | ~200 |

---

## 3. Two batching regimes: size-bound vs cadence-bound

[§11.1](../mtc-architecture-spec.md#111-lifecycle) step 2 emits a batch on
**cadence (`I` = 2–5 s) or full (`B` = 256), whichever comes first**. Which
trigger fires depends on the arrival rate:

```text
entries accumulated per cadence tick = R × I / 3600

regime is cadence-bound  when  R × I / 3600 < B   (partial batches every I seconds)
regime is size-bound     when  R × I / 3600 ≥ B   (full batches, rate R/B)

crossover rate:  R* = B × 3600 / I
    I = 5 s  →  R* = 256 × 720   = 184,320 certs/hour
    I = 2 s  →  R* = 256 × 1,800 = 460,800 certs/hour
```

Consequences:

- At **1x** (2.78 certs/s), a 256-entry batch would take 92.2 s to fill —
  far longer than the cadence — so the system actually runs **cadence-bound**:
  `3600 / I` = 720–1,800 partial batches/hour of ~6–14 entries each.
  §21.1's ~40/hour is therefore a *lower bound* on per-batch work (the
  fewest possible commits for the load); the cadence-bound figures are the
  upper bound at 18–45× more commits.
- At **10x** (27.8 certs/s), still cadence-bound: 720–1,800 batches/hour of
  ~56–139 entries each.
- At **100x** (277.8 certs/s), fill time is 0.92 s < `I`, so the system is
  **size-bound**: 3,906 full batches/hour.

Per-batch work (HSM signings, DDB commit transactions, checkpoint writes,
partial-tile rewrites) follows **batches/hour**, which is capped at
`max(R/B, 3600/I)`. Per-entry work (S3 entry writes) follows `R` alone.

Both regimes are tabulated in §4; headroom conclusions in §5 use whichever
regime is worse for the constraint under discussion.

---

## 4. Load at 1x / 10x / 100x

### 4.1 Size-bound model (the §21.1 idealization)

| Quantity | 1x (10K/hr) | 10x (100K/hr) | 100x (1M/hr) |
| --- | ---: | ---: | ---: |
| Entry arrival rate | 2.78/s | 27.8/s | 277.8/s |
| Batches/hour (`R/B`) | 39.06 | 390.6 | 3,906 |
| HSM signings/hour | 39.06 | 390.6 | 3,906 |
| DDB commit tx/hour | 39.06 | 390.6 | 3,906 |
| DDB counter updates/hour | 39.06 | 390.6 | 3,906 |
| S3 entry writes/hour | 10,000 | 100,000 | 1,000,000 |
| S3 entry writes/sec | 2.78 | 27.8 | 277.8 |
| S3 tile writes/hour (~5/commit) | ~195 | ~1,953 | ~19,531 |
| S3 checkpoint writes/hour | 39.06 | 390.6 | 3,906 |
| Batch fill time (`B / rate`) | 92.2 s | 9.2 s | 0.92 s |

### 4.2 Cadence-bound actuals at `I` = 2–5 s (applies at 1x and 10x)

| Quantity | `I` = 5 s | `I` = 2 s |
| --- | ---: | ---: |
| Batches/hour (`3600/I`) | 720 | 1,800 |
| HSM signings/hour | 720 | 1,800 |
| DDB commit tx/hour | 720 | 1,800 |
| S3 tile writes/hour (~4–5/commit) | ~2,880–3,600 | ~7,200–9,000 |
| S3 checkpoint writes/hour | 720 | 1,800 |
| Entries per batch at 1x | ~14 | ~6 |
| Entries per batch at 10x | ~139 | ~56 |

S3 entry writes are unchanged by regime (per-entry, not per-batch).
At 100x the system crosses into the size-bound regime and §4.1 applies.

---

## 5. Binding constraints and headroom

Constraint ceilings from the spec are labeled **[spec]**. Ceilings that come
from AWS documented default quotas — not stated in the spec, cited only to
contextualize headroom — are labeled **[AWS default, external]**.

### 5.1 HSM signing throughput — first structural bottleneck

**[spec]** §14.3 targets < 100 ms p99 per signing. Treating 100 ms as the
worst-case service time of a single serialized signing channel gives a
conservative floor on throughput:

```text
serial channel capacity ≥ 1 / 0.1 s = 10 signings/s = 36,000 signings/hour
```

| Scale | Signings/hour (worst regime) | Utilization of one serial channel |
| --- | ---: | ---: |
| 1x | 720–1,800 (cadence-bound) | 2–5% |
| 10x | 720–1,800 (cadence-bound) | 2–5% |
| 100x | 3,906 (size-bound) | ~10.9% |

Headroom: ≥ 9x at 100x. The size-bound saturation point of a single serial
channel is `36,000 × 256 = 9.2M certs/hour` (~922x baseline). Note the
cadence-bound regime *flat-lines* signing load at `3600/I` regardless of
issuance rate — batching is the reason HSM throughput is not the practical
ceiling it would be in a sign-per-cert design.

### 5.2 DynamoDB transaction limits

Per-batch DDB work: one 2-item `TransactWriteItems` (step 8) plus one
`UpdateItem` on the shared `counter` item (step 3). Worst case across regimes
is 3,906/hour = **~1.1 writes/s** at 100x.

- **[AWS default, external]** `TransactWriteItems` allows up to 100 items per
  transaction; ours uses 2. Transactional writes cost 2× normal WCU; with
  small (<1 KB) items this is ~4 WCU per commit, ~4.4 WCU/s at 100x —
  negligible against the 40,000 WCU default table-level ceiling.
- **[AWS default, external]** The `counter` item is a single hot key (every
  batch mutates the same item). Per-item throughput caps at ~1,000 WCU/s;
  at ~1.1 updates/s utilization is ~0.1%. This hot key, not table
  throughput, is DDB's eventual ceiling — it binds only near ~1,000
  batches/s (≈ 3.6M batches/hour, far beyond any modeled scale).

### 5.3 S3 request rates

Worst-case write mix at 100x: 277.8 entry PUT/s + ~5.4 tile PUT/s +
~1.1 checkpoint PUT/s ≈ **284 PUT/s**.

- **[AWS default, external]** S3 sustains ~3,500 PUT/s *per prefix*. Even if
  all entry writes hit one prefix, 100x uses ~8% of that. The layout
  ([§8.1](../mtc-architecture-spec.md#81-s3-layout)) already shards entries
  across prefixes (`entries/000/000/…`), so per-prefix load is lower still
  and the ceiling scales with prefix count.
- Single-prefix saturation would occur near 12.6M entry-writes/hour
  (~1,260x baseline).

### 5.4 Tree growth and tile storage over the retention window — the real long-term axis

Nothing above saturates at modeled scales; what grows without bound is
**storage and object count**, linear in `R × T`. With `T` = 7 years
(61,362 hours at 8,766 h/yr):

```text
N = R × 61,362
```

| Quantity | Formula | 1x | 10x | 100x |
| --- | --- | ---: | ---: | ---: |
| Log entries `N` | `R × 61,362` | 614M | 6.14B | 61.4B |
| Tree depth (levels) | `ceil(log₂ N)` | 30 | 33 | 36 |
| Tile levels | `ceil(log₂₅₆ N)` | 4 | 5* | 5* |
| Level-0 tiles | `N / 256` | 2.40M | 24.0M | 240M |
| Total tiles | `≈ N/256 × 256/255` | 2.41M | 24.1M | 241M |
| Tile storage (hashes) | `≈ N × 32 B × 256/255` | 19.7 GB | 197 GB | 1.97 TB |
| Entry objects | `N` | 614M | 6.14B | 61.4B |
| Entry storage | `N × E` (entry size `E` unspecified in spec) | 614M × E | 6.14B × E | 61.4B × E |
| Checkpoint objects (size-bound) | `N / 256` | 2.40M | 24.0M | 240M |

\* 6.14B and 61.4B leaves exceed 256⁴ ≈ 4.29B, adding a fifth tile level
(and one more tile write per commit — the ~5/commit estimate of §2 absorbs
this).

Notes:

- A full tile is `256 × 32 B = 8 KiB`; total tile bytes ≈ `N × 32.13 B`
  (geometric series over levels: `Σ N×32/256ᵏ = N×32 × 256/255`).
- The spec does not fix an entry object size, so entry storage stays
  parameterized as `N × E`. For intuition only (non-normative): at
  `E = 1 KiB`, 1x/10x/100x give ~614 GB / ~6.1 TB / ~61 TB.
- In the cadence-bound regime, checkpoint object count is per-commit,
  not per-256-entries: at 1x with `I` = 2–5 s, ~44M–110M checkpoint objects
  accrue over 7 years. Checkpoints are small but numerous; object count
  affects lifecycle and inventory operations, and pruning checkpoints are
  retained indefinitely
  ([§15.3](../mtc-architecture-spec.md#153-retention-enforcement)).
- Pruning ([§15](../mtc-architecture-spec.md#15-pruning-and-retention))
  bounds *entry and level-0 tile* storage to the retention window; interior
  tiles needed for proofs are retained naturally by the tile structure.

### 5.5 Constraint summary

| Constraint | Ceiling | Utilization at 100x | Binds at (approx.) |
| --- | --- | ---: | --- |
| HSM serial signing **[spec §14.3]** | 36,000 signings/hr | ~11% | ~9.2M certs/hr (size-bound) |
| S3 entry PUT per prefix **[AWS default]** | 3,500/s | ~8% | ~12.6M certs/hr (single prefix) |
| DDB counter hot key **[AWS default]** | ~1,000 writes/s | ~0.1% | ~3.6M batches/hr |
| DDB table WCU **[AWS default]** | 40,000 WCU | <0.1% | not a practical bound |
| Storage / object count | unbounded, linear | — | cost/ops burden, not a hard limit; bounded by retention `T` |

At every modeled scale the write path has large headroom; the design is
deliberately over-provisioned at baseline. The first structural bottleneck
when scaling far beyond 100x is HSM serial signing throughput, followed by
per-prefix S3 entry-write rates.

---

## 6. Scaling levers per bottleneck

Each lever trades against **issuance latency**: a certificate cannot be
delivered until its batch commits, so worst-case queueing delay before the
pipeline even starts is `min(I, B/rate)`. The issuance latency SLO is
< 10 s p99 ([§20.1](../mtc-architecture-spec.md#201-service-health)),
which caps `I` well below ~10 s minus pipeline time (HSM ≤ 0.1 s p99 plus S3
and DDB round trips); the spec's 2–5 s cadence leaves ≥ 5 s of pipeline
budget.

| Bottleneck | Lever | Effect | Latency / other cost |
| --- | --- | --- | --- |
| HSM signings/hour | Increase `B` (larger batches) | Per-batch rate falls as `R/B` in the size-bound regime | Longer fill time `B/rate`; no effect while cadence-bound (`R×I/3600 < B`) |
| HSM signings/hour | Lengthen cadence `I` | Cadence-bound rate falls as `3600/I` | Adds up to `I` seconds to every issuance; bounded by the <10 s p99 SLO |
| DDB commit tx + counter hot key | Same two levers (per-batch cost) | Identical scaling to HSM signings | Same |
| S3 tile writes | Same two levers (~4–5 writes per commit) | Fewer partial-tile rewrites per hour | Same; also less partial-tile write amplification |
| S3 entry writes | None via batching (strictly per-entry) | — | Lever is prefix sharding, already in the [§8.1](../mtc-architecture-spec.md#81-s3-layout) layout |
| Storage / object count | Shorten retention `T` (configurable, [§15.1](../mtc-architecture-spec.md#151-model)) | Linear reduction in retained entries/tiles | Compliance tradeoff; pruning checkpoints kept forever |

Key structural fact: per-batch costs (HSM, DDB, tiles, checkpoints) scale
with `max(R/B, 3600/I)` while per-entry costs scale with `R`. Batch size and
cadence are therefore levers over the *coordination* plane only; the *data*
plane (entry writes, storage) scales strictly with issuance rate and
retention.

---

## 7. Cost notes (modeled only — cross-referenced to §5 of the spec)

Per the [§1 non-goals](../mtc-architecture-spec.md#1-goals-and-non-goals) and
[§5 cost notes](../mtc-architecture-spec.md#5-compute-platform), these are
modeled production costs for the theoretical deployment; no real AWS spend
occurs (development uses LocalStack + SoftHSM, §18).

| Component | Modeled cost (§5) | Load sensitivity |
| --- | --- | --- |
| CloudHSM | ~$1.50/hr × 3 regions ≈ **~$3,300/month** | **Flat.** Clusters are provisioned per region ([§14.2](../mtc-architecture-spec.md#142-cross-region-key-management)); at ≤ 11% signing utilization (§5.1) no additional HSM capacity is needed through 100x |
| ECS Fargate (write path, ~0.5 vCPU / 1 GB × 3 regions) | ~$50/region/month ≈ ~$150/month | Flat until CPU-bound; not modeled further here |
| Lambda (read path, event glue) | "Rounding error at internal-CA scale" (§5) | Linear in read/request volume |
| S3 / DynamoDB requests and storage | Not priced in §5 | Linear in the §4 write rates and §5.4 storage growth; at ≤ ~284 PUT/s and ~1.1 tx/s even at 100x, request costs remain small next to CloudHSM |

Conclusion: modeled monthly cost is dominated by CloudHSM (~$3,300/month) and
is essentially **flat from 1x through 100x** — the marginal infrastructure
cost of issuing 100x more certificates is limited to load-linear S3/DDB
request and storage charges, which is precisely the economic argument for
Merkle-tree batching over per-certificate signing.

---

## 8. Constants provenance (drift detection)

The figures above depend on constants that will eventually live in code. As of
this writing the repository contains **no Rust crates yet** (docs and planning
only), so the architecture spec is the sole source of truth. This table must
be updated with source links when the constants land in code; the
`ops-capacity-calculator` ticket's tests will then re-derive every numeric
example in this document and fail on drift.

| Constant | Value used here | Spec anchor | Code source (update when it lands) |
| --- | --- | --- | --- |
| Batch size `B` | 256 | [§11.1](../mtc-architecture-spec.md#111-lifecycle) step 2, [§21.1](../mtc-architecture-spec.md#211-capacity-planning-rough) | *none yet — expected in the CA-service batch builder* |
| Batch cadence `I` | 2–5 s | [§11.1](../mtc-architecture-spec.md#111-lifecycle) step 2 | *none yet — expected as CA-service config default* |
| Tile width `W` | 256 | [§2](../mtc-architecture-spec.md#2-background-what-is-mtc) concepts table, [§8.1](../mtc-architecture-spec.md#81-s3-layout) | *none yet — expected in the core MTC library* |
| Hash size `H` | 32 B (SHA-256) | [§22.7](../mtc-architecture-spec.md#227-static-vs-dynamic-dispatch-a-deliberate-boundary) | *none yet — expected in the core MTC library* |
| Retention `T` | 7 years default | [§15.1](../mtc-architecture-spec.md#151-model) | *none yet — expected as pruning-worker config default* |
| HSM signing target | <100 ms p99 | [§14.3](../mtc-architecture-spec.md#143-performance-target) | performance target, not a code constant |
| Issuance latency SLO | <10 s p99 | [§20.1](../mtc-architecture-spec.md#201-service-health) | SLO, not a code constant |

Review checklist when code lands: confirm `B`, `I`, `W`, `H`, `T` match the
values above, replace the italicized placeholders with file links, and wire
the numbers into `ops-capacity-calculator` tests.
