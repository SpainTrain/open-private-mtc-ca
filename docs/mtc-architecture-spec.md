# MTC on AWS: Architecture & Implementation Spec

> **Purpose**: Authoritative architecture reference for an implementation of Merkle Tree Certificates (MTC) running on AWS. Intended for use with Claude Code to break down work into Beads epics and tickets.

> **Scope**: A single-organization internal CA for service-to-service mTLS, with multi-region durability and disaster recovery. **No cosigners** — single-trust-boundary deployment. Architecturally extensible to integrate with existing private CAs via an adapter pattern.

> **Status**: v0.7 draft. Theoretical reference architecture and AI-assisted thought experiment.

> **Cloud portability**: Designed AWS-first but with a deliberate cloud abstraction layer (§9). The architecture is portable to GCP and Azure, and the abstractions also accommodate on-premise deployments and long-tail clouds. See §27.

> **Premise**: While this is a solo exploration project, the architecture is intentionally designed to model a realistic, tier-zero enterprise environment. The goal is to stress-test the draft MTC specification against strict corporate requirements (FIPS 140-2 compliance, multi-region DR, formal verification, regulatory retention) to discover what a production-ready private CA would actually require. The extreme rigor of this specification additionally serves as a strict guardrail system for autonomous development using Claude Code — over-specification is intentional. See §23 for how this rigor translates into agent guardrails.

---

## Table of Contents

1. [Goals and Non-Goals](#1-goals-and-non-goals)
2. [Background: What is MTC](#2-background-what-is-mtc)
3. [Threat Model and Trust Assumptions](#3-threat-model-and-trust-assumptions)
4. [Tech Stack Decisions](#4-tech-stack-decisions)
5. [Compute Platform](#5-compute-platform)
6. [Language and Licensing](#6-language-and-licensing)
7. [High-Level Architecture](#7-high-level-architecture)
8. [Data Model](#8-data-model)
9. [Cloud Abstraction Layer](#9-cloud-abstraction-layer)
10. [Issuance Pipeline: Adapter Pattern](#10-issuance-pipeline-adapter-pattern)
11. [Write Path](#11-write-path)
12. [Read Path](#12-read-path)
13. [Failover and Disaster Recovery](#13-failover-and-disaster-recovery)
14. [HSM Integration](#14-hsm-integration)
15. [Pruning and Retention](#15-pruning-and-retention)
16. [Revocation](#16-revocation)
17. [Admin Surface: CLI and UI](#17-admin-surface-cli-and-ui)
18. [Local Developer Experience](#18-local-developer-experience)
19. [Testing Strategy](#19-testing-strategy)
20. [Observability](#20-observability)
21. [Operational Concerns](#21-operational-concerns)
22. [Type Patterns and Code Quality](#22-type-patterns-and-code-quality)
23. [Agent Harnessing](#23-agent-harnessing)
24. [Implementation Roadmap](#24-implementation-roadmap)
25. [Beads Breakdown Guidance](#25-beads-breakdown-guidance)
26. [Open Questions](#26-open-questions)
27. [Multi-Cloud and On-Premise Considerations](#27-multi-cloud-and-on-premise-considerations)
28. [References](#28-references)

---

## 1. Goals and Non-Goals

### Goals

- Implement an MTC-compliant CA per `draft-ietf-plants-merkle-tree-certs` (currently -03)
- Multi-region active-passive deployment on AWS with explicit DR semantics
- Production-quality operational characteristics: monitoring, alerting, runbooks, chaos testing
- Append-only log integrity guaranteed across region failures
- Support for the signatureless certificate optimization
- Integration with ACME for native issuance
- **Adapter pattern enabling integration with existing private CAs** (AWS Private CA, Cloudflare, Keyfactor, etc.) — not v1, but architecturally accommodated
- HSM-backed signing keys with cross-region key management
- Excellent local developer experience — everything runnable, demoable, and debuggable on a laptop
- Production-grade observability including CA-specific signals
- Full CLI/UI parity for admin operations

### Non-Goals (for this iteration)

- Cosigners. Single-trust-boundary internal CA deployment.
- Public-facing Web PKI. Not building a Chrome-trusted CA.
- Inter-organizational trust (CA and relying parties share trust boundary).
- Automated failover in v1. Manual failover with strong tooling is the v1 target.
- **Actual enterprise production deployment.** This is a reference blueprint meant to evaluate the MTC draft and explore what tier-zero PKI demands; it is not a commercially supported product and is not intended for production deployment by the author.
- **Production cloud spending.** Production costs are modeled (e.g., CloudHSM run rates in §5) to keep the design honest about real-world constraints, but actual development relies entirely on the zero-cost local simulation environment described in §18.

---

## 2. Background: What is MTC

Merkle Tree Certificates redesign certificate issuance to integrate Certificate Transparency directly. Rather than the CA signing each certificate individually and submitting to external CT logs, the CA maintains an append-only Merkle Tree log. Each issuance adds an entry; the CA periodically signs checkpoints (root hashes); certificates are assembled from the entry plus an inclusion proof to a signed checkpoint.

**Why this matters for post-quantum**: ML-DSA signatures are roughly 40x larger than ECDSA. With traditional X.509 + CT, every TLS handshake carries multiple PQ signatures, ballooning bandwidth. MTC's signatureless certificates carry only an inclusion proof (~736 bytes for billions of certs).

**Key spec concepts**:

| Concept | Definition |
|---|---|
| Issuance log | Append-only Merkle Tree of all issued certificate entries |
| TBSCertificateLogEntry | Per-cert log entry; uses public key hash, not raw key |
| Checkpoint | Signed (tree size, root hash) commitment |
| Subtree | Range of consecutive entries [start, end) |
| Inclusion proof | Sibling hashes from leaf to subtree root |
| Landmark | Predistributed subtree hash enabling signatureless verification |
| Tile | Fixed 256-leaf chunk of the tree (per tlog-tiles) |
| Trust Anchor ID | Stable CA identifier (per draft-ietf-tls-trust-anchor-ids) |
| `null_entry` | Spec-defined placeholder filling gaps from abandoned batches |

---

## 3. Threat Model and Trust Assumptions

### What we protect

- Integrity of certificate issuance
- Append-only property of the log (no modification or unaudited deletion)
- Non-equivocation (CA does not show different log states to different relying parties)
- Availability for issuance and verification through single-region failures

### What we assume

- CA infrastructure is trusted by all relying parties (single trust boundary)
- HSM-protected signing keys (binary compromise ≠ key compromise)
- AWS infrastructure boundary trusted (S3, DynamoDB, CloudHSM)
- No external transparency requirement — internal monitoring + self-auditor serves the auditing role

### Out of scope

- Insider attack from a fully-trusted CA operator (HSM controls + key ceremony)
- Quantum cryptanalysis of CA signing keys (ML-DSA migration when ready)
- AWS account compromise (standard governance)

---

## 4. Tech Stack Decisions

This section locks in technology choices to remove ambiguity for Claude Code.

### Backend services

| Concern | Choice | Rationale |
|---|---|---|
| Primary language (services) | **Rust** | See §6. Compiler is a force multiplier for agent correctness; ownership eliminates entire bug classes; formal verification tooling for sensitive paths; FIPS-validated PQ crypto via qux-pqc |
| Async runtime | **Tokio** | De facto standard; mature; matches concurrent-task workload (batch builder, lease renewer, signer, intake) |
| Logging | **`tracing`** + `tracing-subscriber` | Structured, async-aware, integrates with OpenTelemetry |
| Errors | `thiserror` (libraries) + `eyre` / `color-eyre` (binaries) | Standard pattern; rich error chains with colored panics and precise locations for DX; `?` operator everywhere |
| Config | `serde` + `figment` (or `config` crate) | Layered: defaults → file → env → CLI |
| HTTP routing | `axum` | Tokio-native; type-safe extractors; ergonomic; widely adopted |
| Internal RPC (if needed) | `tonic` (gRPC) or `axum` JSON | Default to HTTP+JSON via axum; tonic if needed |
| Testing framework | stdlib `#[test]` + `pretty_assertions` | Built-in; no DSL |
| Property-based testing | `proptest` | De facto standard; richer than `quickcheck` |
| Mocking | `mockall` for trait mocks | Standard, derive-based |
| Time | injected `Clock` trait | Never `SystemTime::now()` directly in production code |
| IDs | `ulid` crate | Lexicographically ordered, time-encoded |
| Database access | hand-rolled with `aws-sdk-rust` | Trait-based abstractions; no ORM |
| Dependency injection | constructor injection via `Arc<dyn Trait>` | Idiomatic; no DI framework |
| CLI | `clap` v4 with derive | Standard, ergonomic, generates great help |
| Templating (admin UI) | `askama` or `maud` | Compile-time templates; works with htmx |
| Serialization | `serde` for JSON/config; manual TLS-presentation for spec types | Spec types use the TLS presentation language as defined in MTC spec |
| Formal verification | `kani` (model checker), optionally `creusot` for refinement types | For lease/epoch protocol and write-path linearization point (§22) |

### Infrastructure as Code

| Concern | Choice | Rationale |
|---|---|---|
| IaC tool (v1) | **AWS CDK** | Type-safe, programmatic, idiomatic for AWS |
| CDK language | **TypeScript** | Canonical CDK language; L2/L3 constructs land here first; richer ecosystem |
| Repo split | `infra/` (TypeScript CDK), `crates/` (Rust workspace) | No type sharing across the boundary |
| Config handoff (TS → Rust) | SSM Parameter Store + env vars | Loose coupling, runtime resolution |
| Multi-cloud IaC (future) | **Pulumi** | If multi-cloud is ever required, Pulumi handles AWS, GCP, Azure with one toolchain |

**Note on CDK language**: TypeScript is the canonical CDK language even though the rest of the repo is Rust. The mild awkwardness of two languages is real but small. L2/L3 construct ergonomics are best in TS, and the community publishes constructs as npm packages. The TS↔Rust boundary is the deployment edge — neither side imports types from the other.

**CDK TypeScript with abstraction in mind**: Even within the CDK code, write provider-specific resources behind small construct-level abstractions where it costs little. For example, separate `MtcLogStorageStack` (which today instantiates S3 + DynamoDB) from `MtcComputeStack` (Fargate + Lambda). This makes a future Pulumi port a structural translation rather than a rewrite.

### Frontend / Admin Surface

| Concern | Choice | Rationale |
|---|---|---|
| Admin UI base | **htmx + Askama (or Maud) + SSE** | Compile-time templates in Rust; no JS toolchain; agent-readable; real-time via SSE |
| Tree visualization | Vanilla JS + SVG (single self-contained file) | Isolates the only "rich" piece |
| Asset embedding | `rust-embed` | Single binary deployment |
| Admin CLI | `clap` v4 (derive) | Standard Rust CLI framework; full parity with UI |
| Output formats | Human + `--output json` | Agent-parseable when needed |

The UI is for ops and demo, not a consumer product. Boring tech is right. If a fancier UI is wanted later, the API contract is already there.

### Storage

| Concern | Choice | Rationale |
|---|---|---|
| Tile/entry/checkpoint storage | S3 with Object Lock + CRR + RTC | Immutable, durable, replicates well |
| Coordination state | DynamoDB Global Tables | Conditional writes + epoch enforcement |
| Caching layer | CloudFront for tiles; in-memory LRU for hot path | Tiles are immutable so caching is safe |

### HSM

| Concern | Choice | Rationale |
|---|---|---|
| HSM (production) | CloudHSM (per-region cluster) | True regional independence |
| HSM (dev/test) | SoftHSM2 via PKCS#11 | Same API as CloudHSM |
| Algorithm (v1) | ECDSA P-256 | CloudHSM support; ML-DSA migration when ready |
| Algorithm (v2) | ML-DSA-65 | Post-quantum; tracks CloudHSM roadmap |

### Compute

| Concern | Choice | Rationale |
|---|---|---|
| Write path | ECS Fargate | Long-lived process; lease holder; HSM connection pool |
| Read path | Lambda + CloudFront | Stateless, bursty, horizontal |
| Event handlers | Lambda | Event-driven, short-lived |
| Workflows | Step Functions | Pruning, multi-step ops |

---

## 5. Compute Platform

### Decision: ECS Fargate for the write path, Lambda for read serving and event glue

The natural instinct is "all serverless via Lambda," but Lambda has friction on the write path:

1. **Lease holder** must renew every 20 seconds. Lambda timer pattern works mechanically but creates observability and reliability complexity.
2. **Batch accumulation** wants in-memory queueing on cadence. Lambda's stateless model forces every event through SQS/DDB.
3. **HSM connection pooling**: cold-starting per invocation adds 100ms+ to every batch.

Lambda is the right answer for the *read* path — proof serving is genuinely stateless and request/response shaped, with an immutable cache layer (tiles) that CloudFront handles brilliantly.

### Component-by-component analysis

| Component | Recommended | Rationale |
|---|---|---|
| CA Service (write path) | ECS Fargate | Long-lived; lease; in-memory queue; HSM pool |
| Proof Serving (read path) | Lambda + CloudFront | Stateless, bursty |
| Adapter services | Lambda or Fargate (depending on adapter) | See §10 |
| Revocation Processing | Lambda | Event-driven |
| Pruning Worker | Lambda + Step Functions | Scheduled, multi-step |
| Self-Auditor | Lambda (scheduled) | Periodic log validation |
| Cleanup | Lambda | Event-driven |
| ACME Endpoint | ECS Fargate (same task as CA) | Co-located with batch builder |
| Admin API + UI | ECS Fargate (same task as CA) | Co-located; serves htmx UI |

### Cost notes

ECS Fargate at ~0.5 vCPU / 1 GB per task × 3 regions ≈ ~$50/region/month. Lambda is rounding error at internal-CA scale. CloudHSM dominates (~$1.50/hr × 3 regions ≈ ~$3,300/month).

These represent modeled production costs for the theoretical enterprise deployment described in the Premise. The local developer experience (§18) uses LocalStack and SoftHSM to ensure zero cloud spend during this exploration; the costs above are documented so the design stays honest about what tier-zero PKI actually demands.

---

## 6. Language and Licensing

### Decision: Rust (for services), TypeScript (for CDK)

### Why Rust for services

The decision is driven by four characteristics that matter specifically for an agent-first, correctness-critical CA implementation:

1. **Compiler as agent guardrail.** Rust's strict compiler catches at compile time many error classes that Go catches at test time or runtime. For an agent iterating on code, the compiler is the fastest, most reliable feedback loop. Ownership, lifetimes, exhaustive `match`, and `Result<T, E>` force agents to confront edge cases the moment they write code, not when tests fail later. Empirically, agent code quality per token is higher in Rust than in less-strict languages.

2. **Ownership eliminates entire bug classes.** No data races (compile-time prevented). No nil pointer panics. No accidental shared mutable state. No buffer overflows. For a long-running service handling cryptographic state, these are not theoretical concerns.

3. **Formal verification is achievable, not aspirational.** Rust has a mature verification ecosystem (Kani, Creusot, Prusti, MIRAI, Loom) that can prove correctness of the most security-critical code paths. We will use this for the lease/epoch protocol and the write-path linearization point. Go has no comparable tooling. See §22.

4. **PQ crypto via FIPS-targeted libraries.** `qux-pqc` provides FIPS 203/204/205-targeted ML-DSA, ML-KEM, and SLH-DSA implementations under BSD-3-Clause, designed for production use in compliance-sensitive environments. RustCrypto provides additional permissively-licensed primitives.

Secondary benefits: native AWS SDK is mature and ergonomic; Tokio's async model maps cleanly to our concurrent-task workload; static binary deployment to ECS Fargate; cargo-lambda for Lambda functions.

### Tradeoffs we accept

- **Slower initial development velocity.** Rust's compile times and borrow checker tax are real, especially in early architecture exploration. Mitigated by the fact that agents are doing most of the actual coding and Rust agents are improving fastest.
- **Smaller talent pool than Go.** Not a concern for a personal exploration project; would be a consideration if this became a team effort.
- **Architecture must be considered earlier.** Rust punishes loose architectural thinking — refactoring is harder than in Go. This forces better design earlier, which we treat as a feature, not a bug.
- **Lambda runtime story is younger than Go's.** `cargo-lambda` works well; the AWS Lambda Rust runtime is officially supported. Less battle-tested than Go but adequate.

### Why not Go (revisited)

The earlier draft of this document recommended Go on the grounds that reference implementations exist in Go (`bwesterb/mtc`, `cloudflare/circl`) and AWS SDK ergonomics. On reflection, both arguments are weaker than they appeared:

- `bwesterb/mtc` is ~3,000 lines we'd reimplement clean-room anyway (we can't reuse it directly because we want different storage abstractions; reading it for design patterns is language-independent).
- `qux-pqc` provides BSD-licensed PQ crypto in Rust with explicit FIPS-validation targeting, which is arguably *better* for compliance posture than CIRCL.
- AWS SDK for Rust is GA since late 2023 and arguably has better type-safety ergonomics than the Go SDK.

The compiler-as-correctness-tool argument and formal verification availability tip the decision toward Rust. For a CA where coordination bugs could cause overlapping index ranges (catastrophic) and where cryptographic correctness is paramount, the type system pays for itself.

### Approach to mtc-bridge

We **cannot fork** mtc-bridge due to AGPL. Approach:
1. Read mtc-bridge code for design patterns (especially the SCT-extension stapling for TLS integration). Language differences don't impede this.
2. Reimplement spec primitives clean-room in Rust.
3. Build conformance tests from spec test vectors.

This is a 2-3 week investment to recreate spec primitives in Rust, with the upside that we don't carry forward design choices we wouldn't have made.

### Licensing constraints

| Project | License | Usability |
|---|---|---|
| DigiCert mtc-bridge | **AGPL-3.0** | ❌ Cannot fork. Read for design patterns only. |
| `bwesterb/mtc` (Go) | BSD-3-Clause | ✅ Read for reference patterns; reimplement primitives in Rust. |
| `qux-pqc` | BSD-3-Clause | ✅ FIPS-targeted PQ crypto (ML-DSA, ML-KEM, SLH-DSA). |
| RustCrypto crates | MIT/Apache-2.0 | ✅ ECDSA, SHA-2, X25519, etc. |
| Tokio | MIT | ✅ Async runtime |
| `aws-sdk-rust` | Apache-2.0 | ✅ AWS clients |
| `axum` | MIT | ✅ HTTP framework |
| `clap` | MIT/Apache-2.0 | ✅ CLI |
| `serde` | MIT/Apache-2.0 | ✅ Serialization |
| `tracing` | MIT | ✅ Structured logging |
| `eyre` + `color-eyre` | MIT/Apache-2.0 | ✅ Binary error reporting; colored panics; precise locations |
| `thiserror` | MIT/Apache-2.0 | ✅ Library error types |
| `arbitrary` | MIT/Apache-2.0 | ✅ Structured fuzzing input |
| `cargo-fuzz` | MIT | ✅ libFuzzer-based fuzz testing |
| `proptest` | MIT/Apache-2.0 | ✅ Property testing |
| `kani` | Apache-2.0/MIT | ✅ Formal verification |
| `askama` / `maud` | MIT/Apache-2.0 | ✅ Templating |
| AWS CDK (TypeScript) | Apache-2.0 | ✅ IaC |
| htmx | BSD-2-Clause | ✅ |

---

## 7. High-Level Architecture

Three regions deployed identically; one designated primary at any moment.

```
                    ┌─────────────────────────────────────────┐
                    │         Control Plane (Route 53)        │
                    │  - Health checks per region             │
                    │  - DNS-based read routing               │
                    │  - Failover orchestration               │
                    └─────────────────────────────────────────┘
                              │           │           │
              ┌───────────────┘           │           └────────────────┐
              ▼                           ▼                            ▼
    ┌──────────────────┐       ┌──────────────────┐         ┌──────────────────┐
    │  us-east-1       │       │  us-west-2       │         │  eu-west-1       │
    │  PRIMARY         │       │  STANDBY         │         │  STANDBY         │
    │                  │       │                  │         │                  │
    │  ECS Fargate     │       │  ECS Fargate     │         │  ECS Fargate     │
    │  (CA Service)    │       │  (idle/standby)  │         │  (idle/standby)  │
    │   ├ ACME / API   │       │                  │         │                  │
    │   ├ Adapter API  │       │                  │         │                  │
    │   ├ Admin UI/API │       │                  │         │                  │
    │   ├ Batch builder│       │                  │         │                  │
    │   ├ Tree updater │       │                  │         │                  │
    │   ├ Checkpointer │       │                  │         │                  │
    │   └ Lease renewer│       │                  │         │                  │
    │                  │       │                  │         │                  │
    │  Lambda          │       │  Lambda          │         │  Lambda          │
    │   (proof + glue) │       │   (proof + glue) │         │   (proof + glue) │
    │                  │       │                  │         │                  │
    │  CloudHSM        │       │  CloudHSM        │         │  CloudHSM        │
    │                  │       │                  │         │                  │
    │  S3 (primary)    │◄─────►│  S3 (replica)    │◄───────►│  S3 (replica)    │
    │                  │  CRR  │                  │   CRR   │                  │
    │  DDB Global Tbl  │◄─────►│  DDB Global Tbl  │◄───────►│  DDB Global Tbl  │
    └──────────────────┘       └──────────────────┘         └──────────────────┘
```

### Component responsibilities

| Component | Responsibility |
|---|---|
| ECS Fargate (primary) | Accept issuance via ACME or adapter API; batch; allocate indices; tree update; sign checkpoint; commit |
| ECS Fargate (standby) | Lease-aware idle; ready for promotion; reject writes 503 |
| Lambda: proof-server | Serve inclusion proofs and certificate downloads |
| Lambda: revocation-processor | Process revocation requests; sign new revocation lists |
| Lambda: pruning-worker | Periodic pruning (Step Functions) |
| Lambda: self-auditor | Periodic log consistency validation; signs proof of correct operation |
| Lambda: orphan-cleanup | Cleanup of orphan S3 objects from failed transactions |
| Adapter services (Lambda or Fargate) | Bridge external CAs into the log (see §10) |
| CloudHSM | Hold checkpoint signing keys |
| S3 | Immutable tiles, entries, checkpoints, revocations. CRR with RTC. |
| DynamoDB Global Tables | Coordination state |
| Route 53 + ALB | Health checks, DNS routing |

---

## 8. Data Model

### 8.1 S3 layout

All buckets identical structure across three regions. CRR with Replication Time Control (RTC SLA: 99.99% within 15 min, typically subsecond).

```
s3://mtc-log-{region}/
├── checkpoints/
│   ├── 0000000000000256.signed
│   ├── 0000000000000512.signed
│   └── 0000000000001024.signed
│
├── tiles/
│   ├── 0/                            (level 0 = leaves)
│   │   ├── 000/000.tile
│   │   └── ...
│   ├── 1/
│   └── ...
│
├── entries/
│   ├── 000/000/000000.entry
│   └── ...
│
└── revocations/
    └── 0000000000000256.signed
```

### S3 invariants

- Once written, tile/entry/checkpoint bytes never change
- Object names use fixed-width zero-padded format (lexicographic ordering)
- S3 Versioning enabled (defense in depth)
- S3 Object Lock in **Compliance** mode (true append-only)
- Lifecycle policies handle pruning deletion only after Object Lock retention expires

### 8.2 DynamoDB schema

Single Global Table: `mtc-log-coordination`. Replicas in all three regions.

```
PK: String     SK: String

Items by SK pattern:

┌────────────────────────────┬──────────────────────────────────────────────┐
│ SK                         │ Attributes                                   │
├────────────────────────────┼──────────────────────────────────────────────┤
│ counter                    │ next_index, epoch                            │
│ primary-region-lease       │ region, expires_at, epoch, holder_id         │
│ latest-checkpoint          │ tree_size, s3_key, signed_at, epoch          │
│ latest-revocation          │ tree_size, s3_key, signed_at, epoch          │
│ batch#{batchId}            │ status, start_index, end_index, leaf_count,  │
│                            │ epoch, created_at, committed_at,             │
│                            │ source_type, source_id                       │
│ audit#{tree_size}          │ self-auditor proof of correct operation      │
└────────────────────────────┴──────────────────────────────────────────────┘

PK pattern: log#{logId}
```

Note: `source_type` and `source_id` on batch items support the adapter pattern (see §10).

### 8.3 Lease semantics

- Renewed every 20s by holder; 60s TTL
- Standby regions monitor but do not write
- Expiry beyond 60s safety margin makes lease takeover-eligible
- Every takeover atomically increments `epoch`
- Every write includes `epoch` in ConditionExpression — old primaries cannot write after epoch advance

### 8.4 In-memory state (CA Service)

- **Tile cache**: LRU; tiles are immutable, caching unconditionally safe
- **Checkpoint cache**: last N checkpoints by tree size
- **Entry intake queue**: in-memory, drained by batch builder
- **Counter view**: optimistic local view; authoritative read during UpdateItem

---

## 9. Cloud Abstraction Layer

The CA service is built against a small set of cloud-agnostic interfaces, not directly against AWS SDK calls. This is a v1 design choice — not future work — because:

1. **Cleaner architecture.** Separating "what we need" from "where we get it" forces clear thinking about the actual capabilities the design depends on.
2. **Better testability.** Pure-memory implementations of the interfaces make unit tests faster and more deterministic than LocalStack-backed integration tests.
3. **Cleaner local dev.** A single pure-memory implementation removes the LocalStack dependency from many dev workflows.
4. **Future portability.** GCP, Azure, on-premise, and long-tail cloud support become incremental additions rather than a rewrite. See §27.

### 9.1 The interfaces

The CA service depends on four cloud capabilities. Each maps to a small Rust trait:

```rust
// crates/cloud/src/lib.rs

use async_trait::async_trait;
use std::time::SystemTime;

/// ObjectStore: S3 / GCS / Azure Blob / MinIO / on-prem
#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &str, data: &[u8], opts: PutOptions) -> Result<(), Error>;
    async fn get(&self, key: &str) -> Result<Vec<u8>, Error>;
    async fn head(&self, key: &str) -> Result<Metadata, Error>;
    async fn list(&self, prefix: &str) -> Result<Vec<ObjectInfo>, Error>;
    async fn delete(&self, key: &str) -> Result<(), Error>;
}

/// ObjectLock: S3 Object Lock / GCS Object Retention / Azure Immutable Storage
/// Provides storage-layer append-only enforcement for the retention window.
#[async_trait]
pub trait ObjectLock: Send + Sync {
    async fn put_with_retention(
        &self,
        key: &str,
        data: &[u8],
        retain_until: SystemTime,
    ) -> Result<(), Error>;
    async fn extend_retention(
        &self,
        key: &str,
        new_retain_until: SystemTime,
    ) -> Result<(), Error>;
    async fn get_retention(&self, key: &str) -> Result<SystemTime, Error>;
}

/// ReplicatedKV: DynamoDB Global Tables / Firestore / Cosmos DB / Etcd / Postgres+CDC
/// Provides conditional writes and atomic transactions across replicas.
#[async_trait]
pub trait ReplicatedKv: Send + Sync {
    async fn get(&self, key: &Key) -> Result<Item, Error>;
    async fn put(&self, key: &Key, value: Value, conditions: &[Condition]) -> Result<(), Error>;
    async fn atomic_update(
        &self,
        key: &Key,
        expr: UpdateExpression,
        conditions: &[Condition],
    ) -> Result<Item, Error>;
    async fn transact(&self, ops: Vec<Operation>) -> Result<(), Error>;
    async fn query(&self, prefix: &str) -> Result<Vec<Item>, Error>;
}

/// HSM: CloudHSM / Cloud HSM / Azure Managed HSM / on-prem PKCS#11 / SoftHSM
#[async_trait]
pub trait Hsm: Send + Sync {
    async fn sign(&self, key_handle: &KeyHandle, data: &[u8]) -> Result<Vec<u8>, Error>;
    async fn get_public_key(&self, key_handle: &KeyHandle) -> Result<PublicKey, Error>;
    async fn generate_key(&self, spec: KeySpec) -> Result<KeyHandle, Error>;
}
```

### 9.2 What's NOT in the abstraction

These remain provider-specific because they're either deployment concerns or where abstraction adds complexity without value:

- **Compute platform** — Fargate vs Cloud Run vs Container Apps is a deployment choice; the binary doesn't care.
- **Event/scheduler triggers** — invoked from outside the CA service; mapped at the deployment boundary.
- **CDN configuration** — purely infrastructure concern.
- **DNS / health check configuration** — infrastructure concern.
- **IAM specifics** — handled at the deployment boundary; service code uses opaque credentials/clients.

The principle: abstract **inside** the service binary, not the deployment topology. The CA service binary is portable. The IaC and operational topology are cloud-specific.

### 9.3 Crate layout

The cloud abstraction is organized as a workspace with one crate per backend, all implementing the traits defined in `cloud-types`.

```
crates/
├── cloud-types/           # Trait definitions, error types, common DTOs
│   └── src/
│       ├── lib.rs
│       ├── object_store.rs
│       ├── object_lock.rs
│       ├── replicated_kv.rs
│       ├── hsm.rs
│       └── errors.rs
│
├── cloud-memory/          # Pure-memory implementation for tests + local dev
│   └── src/lib.rs
│
├── cloud-aws/             # AWS implementation (v1 production)
│   └── src/
│       ├── lib.rs
│       ├── s3_object_store.rs
│       ├── s3_object_lock.rs
│       ├── ddb_replicated_kv.rs
│       └── cloudhsm.rs
│
├── cloud-localstack/      # LocalStack-targeted AWS impl (dev/test)
│   └── src/lib.rs         # thin wrappers configuring SDK endpoints
│
└── cloud-softhsm/         # PKCS#11-based HSM for dev (any cloud)
    └── src/lib.rs
```

Future crates:
```
├── cloud-gcp/             # GCS, Firestore, Cloud HSM (post-v1)
├── cloud-azure/           # Blob, Cosmos DB, Managed HSM (post-v1)
└── cloud-onprem/          # MinIO, Etcd or Postgres, on-prem PKCS#11 (post-v1)
```

The CA service depends only on `cloud-types`; concrete backends are wired via Cargo features and runtime configuration.

### 9.4 Wiring

A `Backend` factory wires trait objects from configuration:

```rust
pub struct Backend {
    pub object_store:  Arc<dyn ObjectStore>,
    pub object_lock:   Arc<dyn ObjectLock>,
    pub replicated_kv: Arc<dyn ReplicatedKv>,
    pub hsm:           Arc<dyn Hsm>,
}

pub async fn build_backend(cfg: BackendConfig) -> Result<Backend, Error> {
    match cfg.provider {
        Provider::Aws        => cloud_aws::build(cfg.aws).await,
        Provider::Memory     => Ok(cloud_memory::build()),
        Provider::Localstack => cloud_localstack::build(cfg.localstack).await,
        // future: Gcp, Azure, OnPrem
    }
}
```

The CA service constructor takes a `Backend` and never names a provider:

```rust
impl CaService {
    pub fn new(backend: Arc<Backend>, config: ServiceConfig) -> Self { ... }
}
```

Trait objects (`Arc<dyn Trait>`) carry a small dynamic-dispatch cost but allow runtime backend selection without pervasive generics.

### 9.5 Capability requirements

The interfaces declare the minimum required capabilities. Any backend that supports them can host the CA. The capability bar is:

| Capability | Required behavior | Why |
|---|---|---|
| Object durability | Eleven 9s practical durability | Log integrity |
| Object immutability | Bytes never change after write | Append-only invariant |
| Object retention lock | Cannot delete during retention window even by admins | True append-only at storage layer |
| Cross-region replication | Replication with bounded lag | DR |
| Conditional KV writes | Atomic compare-and-swap on attributes | Lease/epoch protocol |
| KV transactional writes | Atomic multi-item update | Linearization point of step 8 |
| KV cross-region replication | Eventually consistent multi-region | Coordination state replication |
| HSM signing | FIPS 140-2 Level 3 (or equivalent) | Key protection |
| HSM cross-region key access | Key available wherever primary is | Failover |

A backend that lacks any of these isn't a fit. Most cloud and on-prem options support them all; the abstractions don't paper over genuinely missing capabilities.

### 9.6 Local-dev benefits

The pure-memory backend is the cleanest unlock from this design:

- Unit tests run with **zero external dependencies** — no LocalStack, no SoftHSM, no Docker.
- Property-based tests can run thousands of iterations quickly.
- Many adapter and CA service tests don't need full integration setup.
- Onboarding gets simpler: clone, `cargo test`, see green.

The LocalStack-based dev environment remains valuable for end-to-end testing and for exercising the AWS-specific integration code, but it's no longer the only path.

### 9.7 Testing the abstractions

Each trait has a **shared test suite** in `cloud-test-suite` that any implementation must pass:

```rust
// crates/cloud-test-suite/src/object_store.rs
pub async fn run_object_store_suite<F, Fut, S>(factory: F)
where
    F:   Fn() -> Fut,
    Fut: Future<Output = S>,
    S:   ObjectStore + 'static,
{
    test_put_and_get(&factory).await;
    test_overwrite_fails(&factory).await;
    test_list_with_prefix(&factory).await;
    // ...
}

// crates/cloud-aws/tests/object_store.rs
#[tokio::test]
async fn test_s3_object_store() {
    cloud_test_suite::run_object_store_suite(|| async {
        cloud_aws::S3ObjectStore::new(test_config()).await
    }).await;
}

// crates/cloud-memory/tests/object_store.rs
#[tokio::test]
async fn test_memory_object_store() {
    cloud_test_suite::run_object_store_suite(|| async {
        cloud_memory::MemoryObjectStore::new()
    }).await;
}
```

This guarantees behavioral consistency across implementations. Future GCP/Azure backends are validated by the same suite.

### 9.8 Cost of this abstraction

Honest accounting: the cost is real but small.

- **Code surface**: ~4 small interfaces, ~20-30 methods total
- **Mapping AWS SDK to traits**: a thin wrapper layer; mostly type translation
- **Loss of AWS-specific features**: capabilities not in the interface aren't accessible. We don't currently need them; if we ever do, extending the interface is straightforward.
- **Mental overhead**: small. Once the team is used to it, the abstraction is invisible.

The benefit (cleaner architecture, faster tests, multi-cloud accommodation) substantially exceeds the cost.

---

## 10. Issuance Pipeline: Adapter Pattern

The architecture splits issuance into two stages:

```
┌────────────────────────────────────────┐    ┌────────────────────────────────────────┐
│  Stage 1: Source-Specific Intake       │    │  Stage 2: Log-the-Entry Pipeline       │
│  (Adapters)                            │    │  (Common)                              │
│                                        │    │                                        │
│  ┌─────────────────────────────────┐   │    │  ┌────────────────────────────────┐   │
│  │ Native ACME Endpoint            │   │    │  │ Entry intake queue             │   │
│  │  (validates, issues, builds     │───┼────┼─▶│ (TBSCertificateLogEntry)       │   │
│  │   TBSCertificateLogEntry)       │   │    │  └─────────────┬──────────────────┘   │
│  └─────────────────────────────────┘   │    │                ▼                       │
│                                        │    │  ┌────────────────────────────────┐   │
│  ┌─────────────────────────────────┐   │    │  │ Batch builder                  │   │
│  │ AWS Private CA Adapter          │   │    │  └─────────────┬──────────────────┘   │
│  │  (subscribes to issuance events,│───┼────┼─▶              ▼                       │
│  │   re-encodes as MTC entry)      │   │    │  ┌────────────────────────────────┐   │
│  └─────────────────────────────────┘   │    │  │ Tree updater                   │   │
│                                        │    │  └─────────────┬──────────────────┘   │
│  ┌─────────────────────────────────┐   │    │                ▼                       │
│  │ Cloudflare PCA Adapter          │   │    │  ┌────────────────────────────────┐   │
│  │  (polls API, re-encodes)        │───┼────┼─▶│ HSM signer                     │   │
│  └─────────────────────────────────┘   │    │  └─────────────┬──────────────────┘   │
│                                        │    │                ▼                       │
│  ┌─────────────────────────────────┐   │    │  ┌────────────────────────────────┐   │
│  │ Keyfactor Adapter               │   │    │  │ Commit (S3 + DDB)              │   │
│  │  (webhook receiver, re-encodes) │───┼────┼─▶│ ⭐ Linearization point         │   │
│  └─────────────────────────────────┘   │    │  └────────────────────────────────┘   │
│                                        │    │                                        │
│  ... future adapters ...               │    │                                        │
└────────────────────────────────────────┘    └────────────────────────────────────────┘
```

### 10.1 Why this matters

In v1, only the native ACME endpoint exists. But many organizations have existing private CA infrastructure (AWS Private CA, Keyfactor, Cloudflare's PCA, EJBCA, etc.) and want MTC's transparency benefits without ripping and replacing. The adapter pattern lets those CAs continue to issue while feeding MTC entries into our log — similar to how Cloudflare's MTC playground re-encodes existing X.509 certs from trusted CAs as MTCs.

This is **not v1**. But the architecture is designed to admit it without major refactoring later.

### 10.2 Architectural seam

The boundary between Stage 1 and Stage 2 is the **`LogEntry` interface** at the entry intake queue:

```go
// Source-agnostic entry submission
type LogEntry struct {
    TBSCert        []byte           // serialized TBSCertificateLogEntry
    SourceType     SourceType       // "native-acme", "aws-pca-adapter", etc.
    SourceID       string           // adapter's external reference
    SubmittedAt    time.Time
}

// Adapters call this; native ACME calls this; everything funnels through here
type EntryIntake interface {
    SubmitEntry(ctx context.Context, entry LogEntry) (Index, error)
}
```

The `source_type` and `source_id` fields are persisted on the batch state item, providing audit traceability back to the originating issuance event.

### 10.3 Adapter responsibilities

Each adapter is responsible for:

1. Subscribing/polling/listening to its source CA's issuance events
2. Validating the certificate is one we should log (policy decision)
3. Constructing a spec-compliant `TBSCertificateLogEntry` from the source cert
4. Submitting via the `EntryIntake` interface
5. Handling source-specific authentication, retry, and idempotency

Adapters can run as Lambda functions (event-driven sources) or Fargate tasks (long-poll or WebSocket sources). They authenticate to the CA Service via short-lived credentials issued by the platform's IAM.

### 10.4 v1 implication

The internal API and queue interface must be source-agnostic from day one. The native ACME endpoint is implemented as **one adapter among future many**, not as a special integrated path.

This is a small refactor relative to making it special-cased; doing it right in v1 means future adapters are pure additions.

---

## 11. Write Path

The write path executes only on the primary region. Standby regions reject write requests with HTTP 503 + redirect.

### 11.1 Lifecycle

```
┌────────────────────────┐
│ Entry intake           │  ACME endpoint OR adapter calls EntryIntake.SubmitEntry
│ (LogEntry submission)  │
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 1. Lease check         │  Verify we hold current lease; capture epoch
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 2. Batch assemble      │  Accumulate entries; emit on cadence (2-5s) or full (256)
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 3. Allocate indices    │  UpdateItem on counter; ConditionExpression epoch = :epoch
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 4. Persist batch state │  Write batch state to DDB as "pending"
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 5. Write entries       │  N parallel S3 PutObject (entries/.../NNNNNN.entry)
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 6. Tree update         │  Compute affected interior nodes; write new tiles to S3
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 7. Sign checkpoint     │  HSM signs over (tree_size, root_hash, timestamp)
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 8. COMMIT ⭐           │  LINEARIZATION POINT
│                        │  S3 PutObject for checkpoint (deterministic, idempotent)
│                        │  DDB TransactWriteItems:
│                        │   - update latest-checkpoint pointer
│                        │   - mark batch committed
└────────┬───────────────┘
         ▼
┌────────────────────────┐
│ 9. Assemble & deliver  │  Build MTC certificate (TBSCert + MTCProof + CA sig)
│                        │  Return via ACME finalize OR notify adapter
└────────────────────────┘
```

### 11.2 Critical invariants

- Step 8 is the linearization point
- Counter never decreases (abandoned indices become permanent gaps filled with `null_entry`)
- Epoch in every conditional write
- S3 first, DDB second (orphan S3 objects harmless)

### 11.3 Failure modes

| Step | Failure | Effect | Recovery |
|---|---|---|---|
| 1 | Lease check fails | 503 | Client retries against new primary |
| 3 | Counter UpdateItem fails | Lost lease mid-batch | Stand down; batch abandoned |
| 5 | S3 entry partial failure | Some entries written | Retry remaining (immutable, idempotent) |
| 7 | HSM signing fails | No checkpoint | Retry with backoff; alert if persistent |
| 8 | DDB transaction fails | Orphan S3 object | Lifecycle cleans up |

### 11.4 Pseudocode (Rust, async)

```rust
pub struct CaService {
    region:        Region,
    holder_id:     HolderId,
    current_epoch: AtomicU64,
    storage:       Arc<dyn Storage>,
    hsm:           Arc<dyn Hsm>,
    intake:        mpsc::Receiver<LogEntry>,  // source-agnostic
    tile_cache:    Arc<TileCache>,
    clock:         Arc<dyn Clock>,
    log_id:        LogId,
}

impl CaService {
    pub async fn issue_batch(&self, batch: Vec<LogEntry>) -> Result<(), IssueError> {
        // Step 1: lease check
        let lease = self.storage.read_lease().await?;
        if lease.region != self.region {
            return Err(IssueError::NotPrimary);
        }
        if lease.expires_at < self.clock.now() + Duration::from_secs(5) {
            return Err(IssueError::LeaseExpiringSoon);
        }
        let epoch = lease.epoch;
        self.current_epoch.store(epoch.into(), Ordering::SeqCst);

        // Step 3: allocate indices
        let (start, end) = self
            .storage
            .allocate_indices(batch.len(), epoch)
            .await?;

        // Step 4: persist batch state
        let batch_id = BatchId::new();
        self.storage
            .persist_batch_state(&batch_id, start, end, BatchStatus::Pending, epoch)
            .await?;

        // Step 5: write entries (parallel)
        self.storage.write_entries(start, &batch).await?;

        // Step 6: tree update
        let (new_root, new_tiles) = self.update_tree(start, &batch).await?;
        self.storage.write_tiles(&new_tiles).await?;

        // Step 7: sign checkpoint
        let cp = CheckpointBuilder::new()
            .log_id(self.log_id)
            .tree_size(end)
            .root_hash(new_root)
            .signed_at(self.clock.now())
            .build()?;
        let sig = self
            .hsm
            .sign(&CHECKPOINT_KEY_HANDLE, &checkpoint_signing_input(&cp))
            .await?;
        let cp = cp.with_signature(sig);

        // Step 8: COMMIT (linearization point)
        self.storage
            .commit_checkpoint(&cp, &batch_id, epoch)
            .await?;

        // Step 9: deliver
        self.deliver_certificates(start, batch, cp).await
    }
}
```

The function uses `?` for error propagation, `await` for async operations, and (importantly) the lease-check-to-commit window is short and lock-free. The `current_epoch` atomic is read-only after initialization within a single batch — if the lease is lost, the next batch's lease check catches it.

---

## 12. Read Path

### 12.1 Verification flow (relying party)

```
1. Parse certificate; verify signatureAlgorithm == id-alg-mtcProof
2. Decode signatureValue as MTCProof
3. Check certificate's serialNumber against revoked-ranges list
4. Reconstruct leaf hash from TBSCertificate
5. Apply inclusion proof to compute expected subtree root
6. Verify CA's checkpoint signature over a subtree containing this entry
   (NO cosigner signatures required)
7. Standard X.509 path validation
```

For signatureless certificates:

```
6'. Verify computed subtree root matches predistributed landmark hash
```

### 12.2 Proof generation (Lambda)

Both primary and standby regions can serve proofs.

```
1. Read latest-checkpoint pointer from local DDB replica
2. Fetch checkpoint object from local S3
3. Identify tiles needed for inclusion path
4. Fetch tiles from local S3 (cached via CloudFront)
5. Construct MTCProof
6. Return
```

### 12.3 Lambda implementation notes

- Provisioned concurrency for proof-server to eliminate cold starts
- Cache S3 + DDB clients at module level
- Tile fetches go through CloudFront

---

## 13. Failover and Disaster Recovery

### 13.1 Detection

- **Route 53 health checks**: HTTPS endpoint per region, every 10s, fail after 3 consecutive
- **Application self-report**: CloudWatch metrics for lease renewal, checkpoint cadence, latency
- **Lease expiry monitoring**: standby regions periodically read lease item

### 13.2 Decision (v1 = manual)

In v1, failover is a deliberate human decision via the admin CLI:

```bash
mtcctl failover initiate --to us-west-2 --reason "primary region outage"
```

The CLI verifies:
1. Primary is unreachable
2. Standby has caught up: latest checkpoint replicated, S3 CRR current
3. New primary selected based on data freshness

### 13.3 Promotion procedure (idempotent)

```rust
impl CaService {
    pub async fn promote(&self) -> Result<(), PromoteError> {
        // 1. Verify local view of log state
        let cp = self.storage.read_latest_checkpoint().await?;
        if !self.verify_checkpoint_signature(&cp) {
            return Err(PromoteError::InvalidCheckpoint);
        }
        if !self.tile_snapshot_consistent(&cp).await? {
            self.wait_for_crr(&cp).await?;
        }

        // 2. Identify and abandon in-flight batches
        let pending = self.storage.query_pending_batches().await?;
        for batch in pending {
            self.storage.mark_batch_abandoned(&batch).await?;
        }

        // 3. Atomically claim lease + increment epoch
        let new_epoch = self
            .storage
            .claim_lease(self.region, &self.holder_id)
            .await?;

        // 4. Update local epoch view
        self.current_epoch.store(new_epoch.into(), Ordering::SeqCst);

        // 5. First batch fills gaps with null_entry
        self.fill_gaps_and_resume().await
    }
}
```

### 13.4 Old primary recovery

Failed primary returns, reads lease, sees different region as holder + higher epoch, stands down. **Split-brain is impossible** because every write is conditional on `epoch = :currentEpoch`.

### 13.5 RTO and RPO

- RPO: zero for committed work
- RTO v1 (manual): 5–15 minutes
- RTO v2 (automated): <2 minutes target

---

## 14. HSM Integration

### 14.1 Keys

| Key | Algorithm (v1) | Algorithm (v2) | Purpose |
|---|---|---|---|
| Checkpoint signing | ECDSA P-256 | ML-DSA-65 | Sign checkpoints |
| Pruning checkpoint | (same) | (same) | Sign pruning declarations |
| Revocation list | (same) | (same) | Sign revocation snapshots |
| Reporting key | ECDSA P-256 | ML-DSA-65 | Sign compliance reports (separate from issuance keys) |

### 14.2 Cross-region key management

**Recommended (Option A): Per-region CloudHSM cluster with key replication.**

- ✅ True regional independence
- ✅ No cross-region HSM dependency on hot path
- ❌ Manual key replication ceremony per rotation
- ❌ More HSM clusters = more cost

### 14.3 Performance target

<100ms p99 for HSM signing.

### 14.4 FIPS validation boundary

FIPS validation is a property of a specific build artifact, not of the source code. The build pipeline must preserve the validation boundary:

- **Pin exact versions** of FIPS-validated crypto libraries (`qux-pqc`, RustCrypto FIPS modules where applicable). Cargo's lockfile guarantees reproducibility.
- **Disable cross-compilation tricks** that could substitute non-validated implementations. Cargo features that swap crypto backends must be explicitly forbidden via `cargo deny` rules.
- **CI gate**: every release artifact runs a FIPS-validation check before publication. If a dependency upgrade silently removes FIPS validation, the build fails.
- **Container image builds** must not strip or optimize away validated code paths. UPX, sccache aggressive caching, and similar optimizations are denied for production builds.
- **HSM operations** are validated end-to-end: when CloudHSM is the backend, FIPS validation comes from the HSM itself; when SoftHSM is used (dev only), the binary is explicitly marked non-FIPS.

Add an `is_fips_validated() -> bool` method to the `Hsm` trait. Compliance reports (§20.3) include this value. A non-FIPS-validated build can never be deployed to production; CI enforces this with an environment-tagged check.

---

## 15. Pruning and Retention

### 15.1 Model

Pruning is recorded as a signed pruning checkpoint — never silent. Default retention: 7 years (configurable).

### 15.2 Mechanics

- Runs only on primary (lease-enforced)
- Pruning checkpoint format: signed declaration of `pruned_range = [start, end) at tree_size T`
- After commit + replication, S3 lifecycle removes leaf objects
- Object Lock retention must have expired
- Interior tree nodes retained as needed (handled naturally by tile structure)

### 15.3 Retention enforcement

- S3 Object Lock in **Compliance** mode
- CRR preserves Object Lock attributes
- Pruning checkpoints retained indefinitely

---

## 16. Revocation

### 16.1 Format

```
RevocationList {
    log_id:     TrustAnchorID,
    tree_size:  uint64,
    revoked:    list of (start_index, end_index) ranges,
    signed_at:  uint64,
    signature:  bytes (over above fields)
}
```

### 16.2 Distribution

- Hourly default; near-real-time for emergency
- CloudFront-fronted S3 + push notification
- Latency target: 99% of relying parties have current list within 15 minutes

### 16.3 Emergency revocation flow

```
1. Operator initiates via CLI or admin UI (justification + incident ref)
2. Lambda revocation-processor generates new revocation list
3. CloudHSM signs the list
4. Written to S3 with deterministic key
5. DDB latest-revocation pointer updates atomically
6. Push notification triggers immediate refresh
```

---

## 17. Admin Surface: CLI and UI

The admin surface has full **CLI/UI parity**. Every operation available in the UI is available in the CLI, and vice versa. Both consume the same admin API.

### 17.1 Why parity matters

- **Operator ergonomics**: scripts, automation, runbooks
- **Agent affordance**: agents script the CLI deterministically; UI automation is hard
- **E2E testing**: tests can drive the CLI to exercise the full system
- **Audit trail**: CLI commands can be logged with full reproducibility

### 17.2 Admin API

The admin API is HTTP+JSON, defined by an OpenAPI spec checked into the repo (`api/admin.openapi.yaml`). Code generation produces:

- Rust server stubs (consumed by the CA service)
- Rust client (consumed by the CLI)
- TypeScript client (consumed by the UI for any non-htmx interactions)
- API documentation

Adding a new admin operation means: update OpenAPI → regenerate → implement server method → wire into CLI command and UI handler. Both surfaces gain the operation simultaneously.

### 17.3 CLI: `mtcctl`

Built with `clap` v4 (derive API). Subcommand structure:

```
mtcctl
├── status          # Show service status, lease, checkpoint
├── log
│   ├── inspect     # Show log state (size, recent batches, etc.)
│   ├── inclusion   # Generate inclusion proof for an index
│   ├── consistency # Generate consistency proof between sizes
│   └── verify      # Verify a certificate against the log
├── cert
│   ├── issue       # Issue a cert via ACME (for testing)
│   ├── lookup      # Forensics: full info on a cert by index
│   └── revoke      # Revoke a cert (privileged)
├── batch
│   ├── list        # List recent batches
│   └── inspect     # Show full batch details
├── lease
│   ├── show        # Current lease holder, expiry
│   └── renew       # Force renewal (dev/test only)
├── failover
│   ├── status      # Failover readiness assessment
│   └── initiate    # Initiate manual failover (privileged)
├── revocation
│   ├── list        # Show current revocation list
│   ├── add         # Add range to revocation list (privileged)
│   └── distribute  # Trigger emergency redistribution
├── prune
│   ├── status      # Pruning watermark and pending work
│   └── run         # Trigger pruning workflow
├── audit
│   ├── run         # Trigger self-auditor on demand
│   ├── history     # Show audit history
│   └── verify      # Independently verify log consistency
├── report
│   ├── issuance    # Issuance log report
│   ├── revocation  # Revocation report
│   └── compliance  # Compliance bundle
├── adapter
│   ├── list        # List configured adapters
│   ├── status      # Per-adapter health and intake rate
│   └── pause       # Pause an adapter (privileged)
├── repl            # Interactive REPL mode
└── completion      # Shell completion (bash, zsh, fish)
```

**Output formats**: human-readable by default; `--output json` for agents and scripts; `--output yaml` for config-style consumption.

**Authentication**: AWS IAM via signed requests; matches the access patterns of the rest of the AWS ecosystem.

**Authorization**: privileged commands require explicit `--confirm` or interactive confirmation (skippable with `--yes` for automation).

### 17.4 UI: htmx + Askama + SSE

The UI is served from the same Fargate task as the CA service, with assets embedded into the binary via `rust-embed`.

**Why htmx**:
- Real-time updates via Server-Sent Events without a JS framework
- Templates are Askama (Jinja-like, compile-time-checked) — agent-readable, no JSX
- No build toolchain
- No state management library
- A handful of attributes (`hx-get`, `hx-post`, `hx-trigger`, `hx-swap`) cover most needs
- Compile-time template checking catches typos and missing fields at build time, not runtime

**UI structure**:

```
crates/admin/
├── src/
│   ├── lib.rs              # mounts UI + API routes (axum)
│   ├── handlers/
│   │   ├── dashboard.rs
│   │   ├── batches.rs
│   │   ├── certs.rs
│   │   ├── lease.rs
│   │   ├── audit.rs
│   │   └── sse.rs          # Server-Sent Events stream
│   └── templates.rs        # Askama template structs
├── templates/
│   ├── layout.html
│   ├── partials/
│   ├── pages/
│   │   ├── dashboard.html
│   │   ├── batches.html
│   │   ├── certs.html
│   │   ├── lease.html
│   │   ├── audit.html
│   │   └── ...
│   └── components/
│       └── tree-viz.html   # SVG tree visualization
└── static/
    ├── htmx.min.js         # vendored
    ├── tree-viz.js         # vanilla JS for tree rendering
    └── styles.css
```

Static assets are embedded into the binary via `rust-embed`.

**Key UI views**:

| View | Purpose |
|---|---|
| Dashboard | Overall health, current state, growth rate |
| Tree visualization | SVG render of recent tree state |
| Batches | List with status, drill-down |
| Cert lookup | Per-cert forensics |
| Lease | Current holder, history, force-renew |
| Audit | Self-auditor history, manual run |
| Adapters | Per-adapter health and intake rate |
| Reports | Generate compliance reports |

**Real-time updates**: SSE streams from the CA service push state changes (new batches, lease events, audit results) into the UI without polling.

### 17.5 Adding a new admin operation: end-to-end

1. Update `api/admin.openapi.yaml` with the new endpoint
2. `make api-gen` regenerates Rust server stubs, Rust client, TS types
3. Implement the handler in `crates/admin/src/handlers/`
4. Add `clap` subcommand in `crates/mtcctl/`
5. Add htmx-driven UI in `crates/admin/templates/`
6. Add E2E test in `tests/e2e/` exercising the CLI
7. Document in CLI help text + UI tooltips

---

## 18. Local Developer Experience

The repo must support **fast onboarding, agent-first development, and confident local iteration**. Target: clone the repo, run one command, and have a working CA in under 60 seconds.

### 18.1 The 60-second demo

```bash
git clone <repo>
cd <repo>
make demo
```

`make demo` brings up:

- **LocalStack** — emulates S3 (versioning + Object Lock + CRR simulation) and DynamoDB (Global Tables simulation)
- **SoftHSM2** — emulates CloudHSM via PKCS#11
- **CA service** in dev mode (faster cadence, looser timing)
- **Sample ACME client** auto-issuing certs in a loop
- **Admin UI** at `localhost:8080`
- **Self-auditor** running periodically

### 18.2 Admin UI: real-time observability for humans

Already covered in §16.4. Always-on view of tree state, batches, lease, audit, certificates.

### 18.3 Multi-region simulation locally

```bash
make demo-multiregion
```

Brings up three local instances simulating us-east-1, us-west-2, eu-west-1 with:

- Simulated CRR replication between LocalStack instances (configurable lag)
- Simulated DynamoDB Global Tables replication
- `make partition-region us-east-1` simulates network partition
- `mtcctl failover initiate --to us-west-2` exercises failover

### 18.4 Time travel

```bash
make time-advance days=400
```

Fast-forwards the simulated clock. The CA service uses an injected `Clock` interface; in dev mode this is a fake clock controllable via admin API.

### 18.5 Deterministic test fixtures

```bash
make fixture-load name=million-entries
make fixture-load name=with-revocations
make fixture-load name=post-pruning
```

Each fixture is a snapshot of LocalStack + DDB state. Useful for reproducing bugs, performance testing, demos.

### 18.6 Interactive REPL

Two flavors:

```bash
make repl              # `evcxr` Rust REPL with crates pre-loaded
mtcctl repl            # CLI-driven REPL — interactive command shell
```

`evcxr` is a Jupyter-compatible Rust REPL useful for ad-hoc exploration. The CLI REPL is more agent-friendly: deterministic, scriptable, output captured. Both are provided; agents prefer the CLI REPL.

### 18.7 Hot reload

`cargo-watch` watches for changes; CA service auto-rebuilds and restarts. Combined with the admin UI, the feedback loop is fast — Rust's debug-mode compile times for incremental changes are typically a few seconds.

### 18.8 Make targets

| Target | Purpose |
|---|---|
| `make demo` | Single-region demo |
| `make demo-multiregion` | Three-region simulated environment |
| `make test` | All tests |
| `make test-unit` | Unit tests only |
| `make test-prop` | Property-based tests with extended runs |
| `make test-conformance` | Spec conformance suite |
| `make test-chaos` | Chaos engineering scenarios |
| `make test-soak` | Long-running soak test |
| `make test-e2e` | End-to-end tests via CLI |
| `make repl` | Interactive Rust REPL (`evcxr`) |
| `make fixture-load name=X` | Load named fixture |
| `make fixture-save name=X` | Snapshot current state as fixture |
| `make time-advance days=N` | Advance simulated clock |
| `make partition-region X` | Simulate network partition |
| `make api-gen` | Regenerate API code from OpenAPI spec |
| `make codemap` | Generate repo code map |
| `make agent-context` | Generate agent context summary |
| `make agent-precheck` | Pre-task verification |
| `make verify-task` | Post-task verification |
| `make journal msg="..."` | Append to decision journal |
| `make lint` | Run linters |
| `make fmt` | Format code |
| `make audit` | Run self-auditor manually |
| `make bench` | Performance benchmarks |
| `make doctor` | Diagnose dev environment |

---

## 19. Testing Strategy

A CA is critical infrastructure. The standard test pyramid is necessary but not sufficient.

### 19.1 Test layers

#### Unit tests
- All crates
- Run on every PR; fast (<30s total)
- No external dependencies (trait objects + in-memory backend)
- Run via `cargo test`

#### Integration tests
- Against LocalStack + SoftHSM
- Cover storage operations, ACME flow, multi-region failover
- Gated behind `--features integration` Cargo flag
- Run via `cargo test --features integration`

#### End-to-end tests
- Drive the CLI against the full demo environment
- Cover demo scenarios verbatim — issue → verify → revoke → prune
- Located in `tests/e2e/`; some written in shell scripts driving `mtcctl`, others in Rust

### 19.2 Property-based tests (`proptest`)

Merkle tree invariants:

- For any tree size N and any leaf 0 ≤ i < N, an inclusion proof verifies
- Consistency proofs verify between any two tree sizes
- Appending preserves all previously-valid inclusion proofs
- Two trees built from same leaf sequence produce identical roots
- Pruning a range and verifying a proof for an unpruned entry still works
- Serialization round-trips: `parse(serialize(x)) == x` for every spec type
- Hash domain separation: leaf hashes never collide with interior node hashes

### 19.3 Wire-format fuzzing strategy

Manual TLS-presentation serialization for spec types is a known risk area. Untrusted bytes from the network must never panic the service. Our strategy layers four techniques:

**Layer 1: Property-based round-trip (`proptest`)**

For every spec type, implement `Arbitrary` and assert `parse(serialize(x)) == x`:

```rust
proptest! {
    #[test]
    fn checkpoint_roundtrip(cp: Checkpoint) {
        let bytes = cp.serialize_tls_presentation();
        let parsed = Checkpoint::parse_tls_presentation(&bytes).unwrap();
        prop_assert_eq!(cp, parsed);
    }
}
```

Run with extended iteration counts in CI (10,000+ cases for spec types).

**Layer 2: Unstructured fuzzing (`cargo-fuzz`)**

Targets accept arbitrary bytes; harness asserts no panic, no UB, no excessive allocation:

```rust
// fuzz/fuzz_targets/parse_checkpoint.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Never panics; may return Err
    let _ = mtc::Checkpoint::parse_tls_presentation(data);
});
```

One target per externally-parseable type: `Checkpoint`, `MTCProof`, `RevocationList`, `TBSCertificateLogEntry`, `Tile`, ACME request body.

**Layer 3: Structured fuzzing (`arbitrary` bridge)**

For types with high structural complexity (where pure-random bytes rarely hit the interesting paths), use `arbitrary` to generate semi-valid inputs:

```rust
fuzz_target!(|input: ArbitraryProofInput| {
    // input is structurally plausible but may violate semantic invariants
    let proof = construct_proof(input);
    let _ = verify_proof(&proof);  // must not panic
});
```

This catches bugs in the *post-parse* verification path that pure byte-level fuzzing misses.

**Layer 4: Differential against reference implementation**

For any parsed input that both `bwesterb/mtc` (Go) and our parser accept, assert structural agreement. For inputs one accepts and the other rejects, log and investigate. This is run in CI as a nightly job because of cross-language tooling cost.

```rust
fn differential_parse(data: &[u8]) {
    let our_result = mtc::Checkpoint::parse_tls_presentation(data);
    let ref_result = call_go_binary("parse_checkpoint", data);

    match (our_result, ref_result) {
        (Ok(a), Ok(b)) => assert_structural_eq(a, b),
        (Err(_), Err(_)) => (),  // both reject; fine
        mismatch => panic!("differential mismatch: {:?}", mismatch),
    }
}
```

**Properties asserted across all four layers**:

- No panic on any input
- No unbounded allocation (parser refuses inputs claiming impossible sizes)
- No infinite loops (parser has bounded depth)
- No information leak via timing (parsing time bounded by input length, not by content)
- Round-trip identity for valid inputs

**Corpus management**: fuzzing corpora are checked into the repo at `fuzz/corpus/`. Bug-finding inputs are minimized and added to the unit-test suite as regression fixtures. Once a panic is fixed, the input that found it is a permanent regression test.

### 19.4 Spec conformance suite

Clean-room test vectors, ongoing CI gate.

### 19.5 Differential testing

For every cert issuance, run two independent verification paths:

- Path A: our normal verification logic
- Path B: a deliberately separate code path (e.g., shell out to a Go binary wrapping `bwesterb/mtc` for cross-language verification)

If they disagree, fail the test. Catches symmetric bugs that affect both issue and verify code paths in our main implementation. The cross-language comparison is particularly valuable because it catches bugs from a language we wouldn't share with our main code.

### 19.6 Cryptographic invariant tests

- Two checkpoints with same tree size have same root hash
- Revoked cert's index, when removed and re-added, fails verification
- Empty tree root hash is well-defined and constant
- HSM never produces signature for malformed input
- Domain separation prefixes applied consistently

### 19.7 Soak tests

Long-running workloads catching memory leaks, Tokio task leaks, lease starvation, cache eviction issues, HSM connection pool exhaustion. CI runs 10-min on PRs; full overnight runs scheduled.

### 19.8 Adversarial / red-team tests

Explicit attempts to break invariants:

- Malformed tiles to proof generator
- Stalled CRR + attempted promotion
- Wrong signatures from "compromised" HSM
- Forced clock skew between regions
- Duplicate index allocation attempts
- Stale-epoch writes
- Replayed old checkpoints
- Crafted public keys (key-hash collision attempts)

### 19.9 Chaos engineering

Named scenarios with pass/fail criteria:

| Scenario | Fault | Pass criterion |
|---|---|---|
| chaos-primary-loss | Kill primary mid-batch | New primary promotes; no committed cert lost |
| chaos-crr-stall | Inject 30-min CRR delay | Failover waits for catchup |
| chaos-ddb-lag | Force DDB Global Tables lag | No split-brain; reads stale not incorrect |
| chaos-hsm-down | HSM unavailable mid-batch | Batch retries; alerts; eventually stands down |
| chaos-split-brain | Two regions both attempt promotion | Exactly one succeeds (epoch invariant) |
| chaos-old-primary-recovery | Failed primary returns | Stands down to read-only |
| chaos-clock-skew | ±60s skew between regions | Lease semantics correct |
| chaos-adapter-flood | Adapter floods entries | Backpressure works; no entries lost |

### 19.10 Cross-version compatibility

Versioned fixtures of historical log states. Compatibility tests.

### 19.11 Performance regression tests

Track p50, p99, p99.9 over time. Fail CI on regression beyond threshold.

### 19.12 Formal verification

Rust's verification ecosystem makes formal verification of critical paths feasible in v1, not just v2.

**Kani (model checker)** is the primary tool. It can directly verify Rust code for absence of panics, integer overflow, and assertion violations. Apply to:

- The lease/epoch protocol implementation (no two regions hold a current-epoch lease simultaneously)
- The write-path linearization point (atomic transition between pre-commit and committed states)
- Merkle tree append/proof primitives (proof verifies for any leaf in any tree size)

**Loom (concurrency model checker)** for any concurrent state in the CA service. Lease renewer + batch builder + intake handlers run concurrently; Loom enumerates interleavings to find race conditions that traditional testing misses.

**Shuttle (randomized concurrency testing)** complements Loom for properties too expensive to model-check exhaustively.

**TLA+** is still an option for higher-level protocol modeling if needed; less likely to be required given Kani operates on the actual implementation.

This is a meaningful differentiator for a CA implementation. Compliance auditors can be shown machine-checked proofs of the most security-critical code paths.

### 19.13 E2E via CLI

Because of CLI/UI parity, E2E tests can be written entirely as shell scripts driving `mtcctl`. This is a powerful agent affordance — agents can author E2E tests in plain shell.

```bash
# tests/e2e/issuance-and-verification.sh
mtcctl cert issue --domain test.example.com > /tmp/cert.pem
serial=$(mtcctl cert lookup --pem /tmp/cert.pem --output json | jq .serial)
mtcctl log inclusion --index $serial --output json | jq .verified | grep true
```

---

## 20. Observability

### 20.1 Service health

#### Structured logging (`tracing`)

JSON output via `tracing-subscriber` with `tracing-bunyan-formatter` or similar. Correlation IDs through the full lifecycle via `tracing` spans. Sensitive data never logged.

Standard fields on every log entry: `timestamp, level, message, service, region, correlation_id, log_id, epoch, batch_id`.

Spans automatically propagate context: a top-level `issue_batch` span contains nested spans for `lease_check`, `allocate_indices`, `tree_update`, `hsm_sign`, `commit`. Each span emits structured timing data consumable by tracing backends.

#### Distributed tracing (OpenTelemetry / X-Ray)

`tracing` integrates natively with OpenTelemetry via `tracing-opentelemetry`. Every issuance traceable end-to-end. HSM, DDB, S3 calls are spans. Tile cache hit/miss instrumented.

Critically: OpenTelemetry is cloud-agnostic. CloudWatch ingests OTEL natively, as do GCP Cloud Trace and Azure Monitor. Same instrumentation code works across clouds.

#### Metrics (CloudWatch + Prometheus-compatible)

| Metric | Unit |
|---|---|
| `issuance_latency_seconds` | histogram |
| `batch_commit_latency_seconds` | histogram |
| `hsm_signing_latency_seconds` | histogram |
| `lease_renewals_total` | counter |
| `lease_renewals_failed_total` | counter |
| `epoch_advances_total` | counter |
| `batches_committed_total` | counter |
| `batches_abandoned_total` | counter |
| `entries_by_source_total` | counter (labeled by source_type) |
| `tile_cache_hits_total` | counter |
| `tile_cache_misses_total` | counter |
| `crr_replication_lag_seconds` | gauge |
| `ddb_replication_lag_seconds` | gauge |

#### SLOs

| SLI | SLO |
|---|---|
| ACME issuance success rate | 99.9% |
| Issuance latency p99 | <10s |
| Lease renewal success rate | 99.99% |
| HSM signing success rate | 99.9% |
| Read path availability | 99.95% |

### 20.2 CA-specific observability

#### Log inventory dashboard

Always-visible: tree size, total certs issued, currently-valid count, issuance/revocation rate, pruning watermark, storage size by category, **per-source intake rates** (when adapters exist), active relying parties.

#### Per-certificate forensics

`mtcctl cert lookup --index N` returns: TBSCert contents, batch (ID, timestamp, source, region of primary at issuance), checkpoint, revocation status, pruning status, served-proof audit trail, source metadata.

#### Relying party telemetry

Anonymized: landmark-age distribution, signatureless→full fallback rate, verification failure breakdown by reason, geographic distribution.

#### Self-auditor

Periodic Lambda that:
1. Reads latest checkpoint pointer
2. Independently fetches tiles, recomputes root
3. Verifies CA signature
4. Compares against history for consistency
5. Verifies recent batches' entries are accessible and produce expected hashes
6. Records `audit#{tree_size}` proof in DDB

If anomalies detected, page immediately and freeze issuance. **Essential for cosigner-free deployment.**

#### Tree visualization

UI render showing recent batches, landmark positions, pruning watermark, revoked ranges, audit checkpoints.

### 20.3 Compliance reporting

| Report | Contents |
|---|---|
| Issuance log | All certs in date range with full audit trail (per-source segmentation) |
| Revocation log | All revocations with justification, operator, timestamp |
| Pruning log | All pruning events with affected ranges |
| Admin actions | All privileged calls with operator identity |
| Self-auditor history | All audit checkpoints |
| Failover events | Region transitions with timestamps and outcomes |
| Key ceremony events | HSM key generation, rotation, replication |
| SLO compliance | Achievement against documented SLOs |
| Adapter activity | Per-adapter intake counts, success rate, lag |

All reports signed by separate reporting key. Generated via `mtcctl report`.

### 20.4 Capacity prediction

Forward-looking metrics on S3, DDB, HSM, tree growth.

### 20.5 Health endpoints

- `/health/liveness` — process alive
- `/health/readiness` — ready to serve
- `/health/primary` — am I primary?
- `/health/audit` — most recent self-auditor result

---

## 21. Operational Concerns

### 21.1 Capacity planning (rough)

At 10K certs/hour:
- ~40 batches/hour at 256/batch
- ~40 HSM signings/hour
- ~40 DDB transactions/hour for commit
- ~10K S3 entry writes/hour
- ~200 S3 tile writes/hour

### 21.2 Runbooks

Each known failure mode has a runbook in `docs/runbooks/`:
- Detection (alerts that fire)
- Initial assessment
- Mitigation steps
- Recovery procedure
- Postmortem template

Required runbooks: primary failure, HSM unavailability, CRR stall, self-auditor anomaly, emergency revocation, pruning failure, suspected key compromise, adapter flood.

---

## 22. Type Patterns and Code Quality

Rust's type system natively provides most of the agent guardrails this section would otherwise have to construct. We document idiomatic patterns rather than fight the language. The result: agent-generated code that fails to uphold the architecture's invariants typically fails to compile, not later in tests.

### 22.1 Newtypes for domain identifiers

Go required us to use distinct type definitions to avoid implicit conversions. Rust does this idiomatically with newtypes:

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Index(pub u64);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TreeSize(pub u64);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Epoch(pub u64);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LogId(String);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BatchId(String);
```

The compiler will refuse to pass an `Epoch` where a `TreeSize` is expected. No agent discipline required; the language enforces it.

### 22.2 Builder pattern via typestate

Rust's typestate pattern proves at compile time that required fields are present:

```rust
pub struct CheckpointBuilder<H = NoHash, T = NoTreeSize, S = NoSignedAt> {
    log_id:    LogId,
    root_hash: H,
    tree_size: T,
    signed_at: S,
    _marker:   PhantomData<(H, T, S)>,
}

pub struct NoHash;
pub struct WithHash([u8; 32]);
// (similar for others)

impl CheckpointBuilder<NoHash, NoTreeSize, NoSignedAt> {
    pub fn new(log_id: LogId) -> Self { ... }
}

impl<T, S> CheckpointBuilder<NoHash, T, S> {
    pub fn root_hash(self, h: [u8; 32]) -> CheckpointBuilder<WithHash, T, S> { ... }
}

// Only this impl is reachable; compile error if called on incomplete builder:
impl CheckpointBuilder<WithHash, WithTreeSize, WithSignedAt> {
    pub fn build(self) -> Checkpoint { ... }
}
```

Calling `.build()` on a builder missing any required field is a compile error, not a runtime error. The agent literally cannot write code that constructs a partial Checkpoint and tries to use it.

### 22.3 Sum types (enums) for closed sets

Rust enums are proper algebraic data types with exhaustive pattern matching:

```rust
pub enum CheckpointEvent {
    Committed { tree_size: TreeSize, root: [u8; 32] },
    Failed    { reason: String },
    Abandoned { batch_id: BatchId },
}

match event {
    CheckpointEvent::Committed { tree_size, root } => { ... }
    CheckpointEvent::Failed    { reason }          => { ... }
    CheckpointEvent::Abandoned { batch_id }        => { ... }
    // No default arm; compile error if a variant is added and not handled
}
```

Adding a new variant to `CheckpointEvent` causes every `match` over it to fail compilation until updated. Exhaustiveness is the language default.

### 22.4 Typestate state machines

Rather than a `Batch` with a `status` field that runtime code must check:

```rust
pub struct PendingBatch   { /* fields */ }
pub struct CommittedBatch { /* fields */ }
pub struct AbandonedBatch { /* fields */ }

impl PendingBatch {
    pub fn commit(self, cp: &Checkpoint) -> Result<CommittedBatch, CommitError> { ... }
    pub fn abandon(self) -> AbandonedBatch { ... }
}

// CommittedBatch has no .commit() method; cannot be re-committed.
// AbandonedBatch has no .commit() method; cannot transition out of abandoned.
```

The compiler enforces transitions. Agents cannot write code that treats a pending batch as committed, because the type doesn't have the methods that committed batches have.

### 22.5 Phantom types for ID disambiguation

Where Go required generic structs with tag types, Rust's phantom types are zero-cost:

```rust
pub struct Id<T: ?Sized> {
    value: String,
    _phantom: PhantomData<T>,
}

pub struct LogTag;
pub struct BatchTag;

pub type LogId   = Id<LogTag>;
pub type BatchId = Id<BatchTag>;
// Different types at compile time; identical at runtime.
```

### 22.6 Result types are the language

Go's "discriminated unions for results" was a workaround. In Rust:

```rust
pub fn allocate_indices(&self, n: usize, epoch: Epoch)
    -> Result<(Index, Index), AllocateError>;

pub enum AllocateError {
    LostLease,
    StorageError(StorageError),
    InvalidBatchSize { requested: usize, max: usize },
}
```

The compiler enforces that callers handle errors via `?`, `match`, or explicit `unwrap()` (which is grep-able and lint-able).

### 22.7 Static vs dynamic dispatch (a deliberate boundary)

Where dispatch happens matters. The wrong choice in either direction is costly:

- **Overuse of `Arc<dyn Trait>`** introduces vtable lookups and heap allocation on every call. Fine at the top of the call graph (initialized once at startup), painful on hot paths.
- **Overuse of generics with trait bounds** explodes compile times and binary size, and complicates trait object storage when you need heterogeneous collections.

We draw a deliberate line:

#### Use `Arc<dyn Trait>` (dynamic dispatch) when:

- The trait represents a **swappable backend selected at runtime** (the four cloud abstractions: `ObjectStore`, `ObjectLock`, `ReplicatedKv`, `Hsm`). One initialization at startup; thousands of subsequent calls. The vtable overhead is negligible relative to network I/O and cryptographic work.
- The trait represents a **dependency we inject for testing** (`Clock`, `MetricsSink`). The flexibility justifies the cost.
- We need **heterogeneous collections** (e.g., a `Vec<Arc<dyn Adapter>>` holding multiple adapter implementations).
- The trait is at an **architectural seam** where we want runtime configurability without recompilation.

#### Use generics with trait bounds (`impl Trait` or `<T: Trait>`) when:

- The dispatch happens on a **hot path** (per-entry tree hashing, per-byte serialization, per-tile cache lookup).
- The implementation is **known at compile time** for the call site (tree updater calling `Sha256` directly via a generic parameter).
- The trait has many **small, frequently-called methods** where vtable overhead would compound.
- The flexibility is **compile-time-only** (e.g., generic over storage backend at the crate level, monomorphized per binary build).

#### Concrete examples in this codebase

| Concern | Choice | Rationale |
|---|---|---|
| `Backend` struct fields | `Arc<dyn Trait>` | Initialized once at startup; flexibility matters more than dispatch cost |
| Tree updater hash function | `<H: Hasher>` generic | Per-leaf and per-node calls; monomorphize to SHA-256 |
| Serialization primitives | generic `<W: Write>` | Hot inner loops; static dispatch matters |
| `Clock` injection | `Arc<dyn Clock>` | Initialized once; called from many call sites |
| Adapter registry | `Vec<Arc<dyn Adapter>>` | Heterogeneous; flexibility required |
| Per-entry codec | generic | Called per entry per batch; static dispatch matters |

**Default for ambiguous cases**: lean toward generics. Switching from generic to `Arc<dyn>` later is mechanical; switching from `Arc<dyn>` to generic can require type-system refactoring.

### 22.8 Repository pattern boundary (no SDK types in domain code)

Domain code must never see vendor SDK types. The cloud abstraction traits (§9) take and return domain types only:

```rust
// ✅ GOOD — domain types in the trait
#[async_trait]
pub trait ReplicatedKv: Send + Sync {
    async fn atomic_update(
        &self,
        key: &Key,                        // domain type
        expr: UpdateExpression,           // domain type
        conditions: &[Condition],         // domain type
    ) -> Result<Item, Error>;             // domain type
}

// ❌ BAD — SDK types leak through the trait boundary
async fn atomic_update(
    &self,
    input: aws_sdk_dynamodb::operation::update_item::UpdateItemInput,
) -> Result<aws_sdk_dynamodb::operation::update_item::UpdateItemOutput, _>;
```

The AWS-specific implementation translates at the boundary — it accepts domain `Condition` values and constructs `aws_sdk_dynamodb::types::AttributeValue` internally, never exposing the SDK type outward. Same for S3, CloudHSM, and any other backend.

**Why this matters for agents**: an agent that sees an SDK type leak through one trait will repeat the pattern. The CA service's domain logic will become coupled to AWS, defeating the cloud abstraction. The rule is: if a trait signature mentions `aws_sdk_*`, the trait is wrong.

The same rule applies in reverse: AWS-implementation code accepts domain types and emits SDK calls; domain types never reach the SDK directly. The translation layer is small and lives in the backend crate.

### 22.9 Lifetimes prevent stale data hazards

The lease/epoch protocol depends on epoch values being consistent within a batch. Rust's lifetime system can encode this:

```rust
pub struct LeaseGuard<'a> {
    region: Region,
    epoch:  Epoch,
    storage: &'a Storage,
}

impl<'a> LeaseGuard<'a> {
    pub async fn allocate_indices(&self, n: usize) -> Result<(Index, Index), Error> {
        // self.epoch is captured; cannot accidentally use a stale epoch
    }
}
```

The borrow checker prevents holding a `LeaseGuard` across an `await` point that might invalidate the lease, surfacing concurrency bugs at compile time.

### 22.10 `#[must_use]` for important results

```rust
#[must_use = "checkpoint commits must be confirmed"]
pub struct CommitConfirmation { /* ... */ }
```

If a caller ignores a `CommitConfirmation`, the compiler emits a warning (or error with strict lint settings).

### 22.11 The `Clock` trait

Equivalent to the Go injected-clock pattern, but enforced by the type system: production code accesses time only through an injected `Arc<dyn Clock>`. Tests inject a `FakeClock` that supports time advancement.

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

// Lint rule: any direct call to `SystemTime::now()` outside test code is denied.
```

### 22.12 Linting setup

`clippy.toml` and `rustfmt.toml` configured with strict settings:

- `clippy::pedantic` — comprehensive stylistic warnings
- `clippy::nursery` — newer, sometimes opinionated lints
- `clippy::cargo` — workspace and dependency hygiene
- `unsafe_code = "forbid"` — no unsafe blocks anywhere (with explicit exceptions for FFI to PKCS#11)
- `missing_docs` — all public items documented
- `clippy::unwrap_used`, `clippy::expect_used` — denied in non-test code
- Custom `dylint` lints for repo-specific patterns (e.g., "no `SystemTime::now()` outside test code")

### 22.13 Required CI checks

Every PR must pass:
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features` (unit + property + integration)
- `cargo test --doc` (doctest examples)
- `cargo deny check` (license + advisory + duplicate dependency checks)
- `cargo audit` (security advisories)
- API spec validates (no breaking changes without explicit override)
- For PRs touching critical paths: `cargo kani` (formal verification proofs pass)

---

## 23. Agent Harnessing

The repo is designed for agent-first development. This section documents the patterns and tooling that enable that.

**Why this rigor for a solo project?** This repository serves as an experiment in AI-driven development of highly complex, mathematically rigorous systems. The strict type discipline (§22), formal verification harnesses (§19.12), over-specified rules (§23.2), and capability-bound trait boundaries (§9) are designed to act as architectural guardrails for Claude Code. The intent is to prevent the agent from making the dangerous compromises that are common in typical AI code generation — silent error swallowing, hidden coupling, type drift, untested concurrency, eroded invariants. The architecture is over-specified on purpose: every guardrail is a place where the agent might otherwise drift, made impossible by the language, the type system, the lints, or the CI gates.

This is a thesis of the project: when a system is structured tightly enough, an autonomous agent can build it correctly. The thoroughness of this section is the testing of that thesis.

### 23.1 `.claude/skills/`

Per-task entry points that orient an agent without requiring it to read the entire repo. Each skill is a markdown file with:

- Goal: what task this skill helps with
- Files involved: pointer to the relevant 3–5 files
- Pattern: example of the standard approach
- Common pitfalls

Suggested skills to seed:

- `add-admin-endpoint.md` — add a new admin operation end-to-end (OpenAPI → server → CLI → UI → test)
- `add-storage-method.md` — extend the Storage interface with a new operation
- `add-metric.md` — add a CloudWatch metric
- `add-chaos-test.md` — write a new chaos scenario
- `add-fixture.md` — capture and document a new test fixture
- `write-runbook.md` — runbook template + how to wire alerts
- `add-adapter.md` — add a new external CA adapter
- `update-spec-version.md` — track a new MTC draft revision

### 23.2 `.claude/rules/`

Repo-specific rules the agent should always follow. Suggested rules to seed (gist only — not full content):

- **`always-test-with-pr.md`** — every PR includes tests; never code without tests
- **`no-systemtime-now-in-prod.md`** — `SystemTime::now()` is forbidden outside test modules; use injected `Clock`
- **`no-unwrap-in-prod.md`** — `unwrap()` and `expect()` forbidden outside tests; use `?` and proper error types
- **`no-unsafe.md`** — `unsafe` blocks forbidden except in documented FFI boundaries (PKCS#11)
- **`use-newtypes.md`** — domain identifiers are newtypes, never type aliases
- **`thiserror-for-libs-eyre-for-bins.md`** — error type discipline; binaries use `eyre` + `color-eyre` for the DX benefit of colored stack traces and precise panic locations
- **`no-sdk-types-in-domain.md`** — vendor SDK types (`aws_sdk_*`, etc.) must never appear in trait signatures or domain code; translation happens inside the backend implementation crate (§22.8)
- **`prefer-generics-on-hot-paths.md`** — use `<T: Trait>` for hot paths; `Arc<dyn Trait>` only at architectural seams or for heterogeneous collections (§22.7)
- **`fips-boundary-preserved.md`** — production builds must pass the FIPS validation CI check; dependency changes that alter the FIPS posture require explicit ADR
- **`cli-ui-parity.md`** — every admin operation must be available in both CLI and UI
- **`document-decisions.md`** — non-trivial decisions go in `docs/adr/`
- **`run-precheck-first.md`** — start every task with `make agent-precheck`
- **`update-codemap-on-structure-change.md`** — `make codemap` after creating/moving crates
- **`single-pr-acceptance.md`** — one PR per task unless explicitly large; otherwise decompose
- **`kani-for-critical-paths.md`** — code touching lease/epoch or write-path linearization should include or update Kani harnesses
- **`spec-pin-and-track.md`** — implementation pins to a specific MTC draft revision; WG GitHub `main` is watched for in-flight changes; non-trivial divergences file Beads tickets

### 23.3 `AGENTS.md`

Top-level guide at repo root covering:
- Repository tour
- How to start a task (precheck, branch, test, journal)
- Standard patterns
- Where to find what
- Common commands
- Gotchas

This is the agent's starting point.

### 23.4 Inner-loop tooling

Within a single agent task:

- **`make agent-precheck`** — verifies environment, reads recent decisions, lints current state, runs fast tests. Run before starting work.
- **`make watch`** — runs lint+test on every save; sub-second feedback.
- **`make verify-task`** — runs acceptance criteria checks before declaring done.
- **`WORKING_SET.md`** — agent maintains during task; lists files touched, decisions made, open questions.

### 23.5 Outer-loop tooling

Across tasks:

- **Decision journal** at `docs/journal.md`. `make journal msg="..."` appends timestamped entries. Future agents read this for context.
- **Beads + journal integration** — closing a Beads ticket auto-appends a journal entry.
- **ADRs** in `docs/adr/` for decisions worth preserving as standalone artifacts.
- **Pattern library** at `docs/patterns/` documenting recurring code patterns with examples. Agents reference these instead of pattern-matching to whatever they see.
- **Anti-patterns** at `docs/anti-patterns.md` — common mistakes to avoid (e.g., "don't use `time.Now()` directly").

### 23.6 Scaling beyond context windows

The repo will grow beyond what fits in any single context. Strategies:

#### Predictable structure

Standard Cargo workspace layout. Each subsystem is its own crate, self-contained, with a clear public API in `lib.rs`. Tests in `tests/` (integration) and inline `#[cfg(test)]` modules (unit). Agents navigate via convention.

#### Hierarchical READMEs

Repo root README points to subsystem READMEs. Each subsystem README states purpose, public API, internal structure, gotchas. Agents read READMEs as cheap context before diving into code.

#### Code map

`make codemap` generates `CODEMAP.md` — a single-file summary of all packages, their exports, dependencies. Updated automatically; checked into repo. Provides agent navigation without loading all files.

Format:

```
CRATE: storage
  PURPOSE: S3 + DDB storage backend with epoch-aware writes
  PUBLIC API:
    - trait Storage
    - struct S3DdbStorage
    - struct FakeStorage (test-only via cfg)
  DEPENDS ON: cloud-types, types, clock
  USED BY: ca-service, admin
```

#### Strict module privacy

Rust's default module privacy is `pub(crate)` unless explicitly exported. Exploit this. Each crate exposes a minimal `lib.rs` API; internal modules are not re-exported. Agents know that the crate's `lib.rs` is the surface to learn deeply, not internal modules.

#### Spec-by-test

Important behavior documented as test names. `grep` test names for "what does X do" finds executable docs.

#### Living architecture diagrams

Mermaid diagrams in `docs/architecture/` render in GitHub. CI flags drift between diagrams and code.

#### Search affordances (pre-built)

So agents don't burn tokens reading files:

- `make find-tests path=X` — relevant test files
- `make find-callers symbol=Y` — callers
- `make find-impl iface=Z` — implementations
- `make find-todo` — TODO comments

#### ADR index

All ADRs in one indexable place. Agents `grep` decisions before re-deciding.

### 23.7 Suggested journal entry template

```markdown
## 2026-04-15 — Implemented epoch-aware counter UpdateItem

**Ticket**: STORAGE-3 (closed)
**PR**: #42

Decisions:
- Used DDB ConditionExpression on `epoch` rather than optimistic locking
- Rejected: in-memory counter cache (would violate single-writer guarantees)
- Used newtype `Epoch(u64)` rather than raw u64 for type safety
- Added Kani harness `verify_no_overlapping_allocations` to crates/storage/proofs/

Open questions:
- Counter contention at high batch rates — needs benchmarking (filed BENCH-1)
- Should we expose `Epoch` in public API? (filed for ADR)
```

### 23.8 Context budget patterns

Strategies for tasks where the relevant context exceeds the window:

- **Skill-driven scoping** — skills target ~5 files
- **`make agent-context`** — produces a token-budgeted summary of: current branch state, recent journal entries, open Beads, dirty files, last test results
- **Beads ticket scoping** — tickets specify exactly which files are in scope
- **Scoped tests** — test for one feature at a time, not the whole suite, when iterating

---

## 24. Implementation Roadmap

Phases sequenced by dependency. Each phase sized into PRs per §24.

### Phase 0: Foundation

- AWS CDK (TypeScript) for AWS resources
- Repository scaffolding (Cargo workspace, CI/CD, linting)
- LocalStack-based dev environment
- Basic admin UI shell (htmx + Askama)
- Basic CLI scaffolding (`clap` v4)
- OpenAPI + code generation pipeline
- Beads board initialized
- `.claude/skills/`, `.claude/rules/`, `AGENTS.md` seeded

### Phase 1: Core MTC Library

- Implement spec primitives clean-room in Rust (refer to `bwesterb/mtc` for design patterns; do not copy code)
- Integrate `qux-pqc` for ML-DSA (gated behind feature flag until production-ready)
- Integrate RustCrypto for ECDSA P-256 (v1 algorithm)
- Domain types using newtypes per §22
- Serialization round-trip tests
- Property-based tests for tree invariants (`proptest`)
- Spec conformance suite scaffolding (test vectors + cross-language differential against bwesterb/mtc Go binary)
- First Kani harnesses for critical tree primitives

### Phase 2: Cloud Abstraction Layer

- `crates/cloud-types/` traits: `ObjectStore`, `ObjectLock`, `ReplicatedKv`, `Hsm`
- Pure-memory implementation (`crates/cloud-memory/`)
- AWS implementation (`crates/cloud-aws/`)
- LocalStack configuration helper (`crates/cloud-localstack/`)
- SoftHSM via PKCS#11 (`crates/cloud-softhsm/`)
- Shared test suites (`crates/cloud-test-suite/`) — same tests run against every backend
- Storage facade built on top of the abstractions; tile cache (LRU)

### Phase 3: Single-Region CA Service (MVP)

- **EntryIntake interface** (source-agnostic) — establishes adapter pattern seam
- ACME endpoint as first adapter
- Batch builder
- Tree updater
- Checkpoint signer (SoftHSM in dev, CloudHSM in prod)
- Admin API + CLI + UI for status/inspection
- Single-region issuance end-to-end via demo

### Phase 4: Read Path

- Lambda proof-server
- CloudFront configuration
- Inclusion proof generation
- Verification library
- CLI commands for proof inspection and verification

### Phase 5: Multi-Region Coordination

- Lease/epoch implementation
- Lease renewer
- Standby read-only mode
- Manual failover (CLI command + UI)
- Multi-region simulation in dev environment
- Chaos test scenarios
- **Kani formal verification of lease/epoch invariants** (not optional; this is core to the trust story)
- Loom-based concurrency test for the lease renewer interacting with batch builder

### Phase 6: Observability

- Self-auditor Lambda
- Metrics, dashboards, SLOs
- Per-cert forensics tooling
- Compliance report generation
- Runbooks
- Tree visualization in UI

### Phase 7: Pruning & Revocation

- Pruning Step Functions workflow
- Revocation Lambda
- Distribution mechanisms
- Object Lock retention validated end-to-end

### Phase 8: Production Hardening

- Full chaos test suite
- Soak testing
- Performance regression tracking
- Security review
- Documentation polish

### Phase 9 (post-v1): Adapter Pattern Implementations

- AWS Private CA adapter
- Reference adapter SDK for third-party CAs
- Adapter health/observability surface

---

## 25. Beads Breakdown Guidance

### 25.1 Epic structure

```
EPIC: Foundation & Infrastructure (CDK, CI/CD, dev env, Cargo workspace)
EPIC: Core MTC Library            (spec primitives in Rust, qux-pqc integration)
EPIC: Cloud Abstraction Layer     (traits + AWS + memory + tests)
EPIC: Storage Facade              (built on top of cloud abstractions)
EPIC: CA Service (Single-Region)
EPIC: Multi-Region Coordination   (includes Kani verification of lease/epoch)
EPIC: Read Path
EPIC: Self-Auditor & Observability
EPIC: Pruning & Retention
EPIC: Revocation
EPIC: Admin Surface (CLI + UI + API)
EPIC: Local Developer Experience
EPIC: Testing Infrastructure      (proptest, fuzz, chaos, Kani, Loom, Shuttle)
EPIC: Agent Harnessing
EPIC: Operational Tooling
EPIC: Adapter Pattern (post-v1)
EPIC: Multi-Cloud Backends (post-v1)
```

### 25.2 PR-based ticket sizing

Sized in **PRs**, not hours.

| Size | Definition |
|---|---|
| **S** | 1 PR, ~50–200 lines net change including tests |
| **M** | 2–3 PRs, each independently shippable |
| **L** | "Needs decomposition" — break down before starting |

### 25.3 PR best practices

- **A PR is a single reviewable unit and a single revertable unit**
- **Target**: 200–500 lines net change including tests. Outliers acceptable when natural (vendored imports, generated code).
- **Vertical slices over horizontal slices**: a PR touching storage + service + handler + tests for one feature is better than one column added to all tables.
- **Tests in the same PR as the code.** Always.
- **Each PR ships independently to main.** No long-lived branches. Use feature flags.
- **Acceptance criteria automatically validated** (test passes, lint passes, manual demo step).
- **Rollback plan** documented in every PR description.

### 25.4 What makes a good ticket

```markdown
## Title: Implement counter UpdateItem with epoch check

### Goal
Add atomic index allocation via DDB UpdateItem with conditional epoch check.

### Acceptance Criteria
- New trait method `Storage::allocate_indices(&self, n: usize, epoch: Epoch) -> Result<(Index, Index), AllocateError>`
- Returns `AllocateError::LostLease` on `ConditionalCheckFailedException`
- Unit tests cover: success path, lost lease, retry behavior
- Property test (`proptest`): 1000 concurrent calls produce non-overlapping ranges
- Integration test against LocalStack DDB
- Kani harness asserting no two successful allocations have overlapping ranges

### Out of Scope
- Lease renewer (separate ticket)
- Counter initialization (separate ticket)

### Dependencies
- DDB-CLIENT-WRAPPER (#42)

### Testing
- Unit, integration, property, Kani

### Demo
After merge: `make demo` → `mtcctl batch list` → see counter advance
```

### 25.5 Critical paths

- **`core-mtc-types`** blocks everything (newtypes, spec primitives, serialization)
- **`cloud-abstraction-traits`** blocks all storage and CA service work — `ObjectStore`, `ObjectLock`, `ReplicatedKv`, `Hsm` traits must exist before backend implementations
- **`memory-backend`** is the second cloud-abstraction priority; unlocks fast unit testing for everything downstream (no LocalStack required)
- **`entry-intake-trait`** must exist before ACME endpoint (establishes adapter seam)
- **`lease-epoch-protocol`** blocks all multi-region work; includes Kani verification harnesses
- **`openapi-codegen-pipeline`** blocks admin UI/CLI work

### 25.6 Parallelizable work

- Storage layer ↔ MTC library (clean interface)
- Read path Lambda ↔ Write path Fargate service
- Admin UI ↔ Admin CLI ↔ Admin API (clean OpenAPI boundary)
- Pruning ↔ Revocation
- Chaos test scenarios (each independent)
- Each fixture
- Documentation ↔ Implementation

### 25.7 Dev experience tickets are first-class

Tickets like "add admin UI tile visualizer" or "add `make time-advance`" are sized and prioritized alongside feature work. Dev experience enables velocity; investing late is too late.

### 25.8 Self-auditor is P0

Self-auditor must ship before any production deployment. It is the substitute for external transparency in our cosigner-free deployment.

### 25.9 Adapter pattern in v1

Even though only the ACME adapter ships in v1, the **`EntryIntake` interface** is in scope from day one. This is a small refactor cost paid up front to avoid a large refactor cost later.

---

## 26. Open Questions

| ID | Question | Resolution Plan |
|---|---|---|
| OQ-1 | When does CloudHSM support ML-DSA? | Track AWS roadmap; ECDSA P-256 in v1 |
| OQ-2 | TLS Trust Anchor IDs draft stability | Track draft; pin and update |
| OQ-3 | ACME extension for MTC | Implement custom server-side |
| OQ-4 | Landmark distribution cadence | Start hourly; tune based on observed RP behavior |
| OQ-5 | Tile cache eviction policy | Start LRU; profile and adjust |
| OQ-6 | DigiCert SCT-stapling pattern adoption | Adopt as reference; document in our impl |
| OQ-7 | Step Functions vs simple Lambda chain for pruning | Decide during pruning epic |
| OQ-8 | Self-auditor cadence | Start hourly; informed by observed log growth |
| OQ-9 | Kani vs Creusot for verification of refinement properties | Kani primary; evaluate Creusot for refinement-typed proofs as needed |
| OQ-10 | `evcxr` REPL adequacy | Provide both `evcxr` and CLI REPL; CLI primary for agents |
| OQ-11 | OpenAPI vs proto for admin API | Default OpenAPI; switch to proto if performance demands |
| OQ-12 | First adapter to implement (post-v1) | Likely AWS Private CA; lowest friction |
| OQ-13 | Async runtime choice — Tokio confirmed | Tokio is the de facto standard; no change anticipated |
| OQ-14 | Differential testing partner — Go binary or alternative? | Use bwesterb/mtc Go binary as differential check; cross-language is a feature |

---

## 27. Multi-Cloud and On-Premise Considerations

The cloud abstraction layer (§9) is built into v1. This section documents what it would take to actually run on GCP, Azure, or on-premise. **None of this is v1 work** — it's the future scenario the v1 design accommodates.

### 27.1 Portability summary by component

| Component | AWS (v1) | GCP | Azure | On-prem / long-tail |
|---|---|---|---|---|
| ObjectStore | S3 | GCS | Blob Storage | MinIO, Ceph, on-prem S3-compatible |
| ObjectLock | S3 Object Lock (Compliance) | GCS Object Retention Lock | Immutable Storage (Locked) | MinIO Object Lock, application-layer enforcement |
| ReplicatedKV | DynamoDB Global Tables | Firestore (multi-region) | Cosmos DB | Etcd, FoundationDB, CockroachDB, Postgres + CDC |
| HSM | CloudHSM (per region) | Cloud HSM | Managed HSM | On-prem PKCS#11 HSM (Thales, Utimaco, etc.) |
| Compute (write path) | Fargate | Cloud Run, GKE | Container Apps, AKS | Kubernetes, bare metal |
| Compute (read path) | Lambda | Cloud Functions, Cloud Run | Functions, Container Apps | Knative, OpenFaaS |
| Workflows | Step Functions | Workflows | Logic Apps | Argo Workflows, Temporal |
| CDN | CloudFront | Cloud CDN | Front Door / CDN | Varnish, Nginx, Fastly |
| Metrics/logs | CloudWatch | Cloud Monitoring/Logging | Monitor/Log Analytics | Prometheus + Grafana + Loki |
| DNS / health checks | Route 53 | Cloud DNS | Traffic Manager | CoreDNS, ExternalDNS |

The architecture has no fundamental coupling to AWS. Every component has at least one credible alternative on every major target.

### 27.2 Portability concerns by capability

#### Object retention lock semantics

All three major clouds support the equivalent of S3 Object Lock in Compliance mode:

- **AWS S3 Object Lock (Compliance)** — cannot be deleted by anyone before retain-until
- **GCS Object Retention Lock** — same semantics; available since 2024
- **Azure Immutable Storage (Locked)** — same semantics

For on-premise, **MinIO** supports S3-compatible Object Lock. Other on-prem stores may require application-layer enforcement, which is weaker than storage-layer enforcement and may not satisfy compliance frameworks. This is a real consideration for on-prem deployments.

#### Conditional writes for lease/epoch

The lease/epoch protocol depends on atomic compare-and-set semantics. All major options support this:

- **DynamoDB**: ConditionExpression on UpdateItem
- **Firestore**: transactions with version-based concurrency
- **Cosmos DB**: ETags and stored procedures
- **Etcd**: native compare-and-swap (this is what Kubernetes is built on)
- **FoundationDB**: transactional everything
- **Postgres**: row-level locking + version columns

The interface abstracts the differences. The semantics are the same.

#### Multi-region replication

This is where clouds differ most:

- **AWS DynamoDB Global Tables**: active-active eventually consistent multi-region. We use this with single-writer discipline (lease) so we never actually depend on conflict resolution.
- **GCP Firestore**: strong consistency within a region, eventual consistency cross-region for multi-region instances. Same model.
- **Azure Cosmos DB**: five consistency models; we'd use bounded staleness or strong.
- **Etcd**: single-cluster strong consistency; multi-region requires explicit clustering across regions or external replication.
- **Postgres**: streaming replication with primary/replica; geographic distribution adds operational complexity.

The "wait for replication catchup" step in failover requires per-backend implementation. This is a small amount of code but each backend has different observability into replication lag.

#### Cross-region replication time guarantees

S3 RTC offers a 15-minute SLA. Equivalents:

- **GCS**: Turbo Replication for dual-region buckets has 15-minute RPO target.
- **Azure**: Geo-Redundant Storage (GRS) has no published SLA; ~15 minutes typical. **No direct SLA equivalent**, requires application-level monitoring of replication lag.
- **MinIO**: configurable replication; SLA is whatever you operate it to.

For Azure deployments, the failover wait window may need to be more conservative since we lack an explicit SLA.

#### HSM differences

| Cloud | HSM | API surface | Notes |
|---|---|---|---|
| AWS | CloudHSM | Direct PKCS#11 | FIPS 140-2 Level 3 |
| GCP | Cloud HSM | KMS-fronted | FIPS 140-2 Level 3; one extra layer of indirection |
| Azure | Managed HSM | PKCS#11 + KMS-style | FIPS 140-2 Level 3 |
| On-prem | Vendor HSM | PKCS#11 standard | FIPS 140-2 Level 3+ depending on model |

The `HSM` interface abstracts cleanly. GCP's KMS-fronted model is a slight friction — the KeyHandle abstraction may need to encode a KMS key resource path rather than a direct HSM slot.

### 27.3 On-premise specifics

Running on-premise (not on any cloud) is a legitimate deployment target. The capabilities map differently:

- **ObjectStore**: MinIO is the most direct fit (S3-compatible API, supports Object Lock). Ceph RGW is an alternative.
- **ReplicatedKV**: Etcd, CockroachDB, or FoundationDB are the strongest fits. Postgres with logical replication and a small CDC layer also works.
- **HSM**: any FIPS 140-2 Level 3 PKCS#11 device. Thales Luna, Utimaco, Entrust, etc.
- **Compute**: Kubernetes. The CA service runs as a StatefulSet in the primary region; Lambda equivalents run as Knative or OpenFaaS functions.
- **Multi-region replication**: harder than in cloud. May require operating two MinIO clusters with bidirectional replication, or a single global storage layer (Ceph multi-site, Spectrum Scale).

The cloud abstraction layer treats on-prem as just another backend. The infrastructure operational complexity is higher (you're operating storage and replication yourself), but the CA service code is unchanged.

### 27.4 Long-tail clouds

Smaller cloud providers (Cloudflare R2, Backblaze B2, Wasabi, OVH, Scaleway, etc.) typically expose S3-compatible object storage. With careful interface implementation:

- **R2/B2/Wasabi**: S3-compatible, often without full Object Lock support — requires verification
- **Cloudflare D1/Workers KV**: candidate for ReplicatedKV but with different semantics; may not support transactional multi-key updates
- **HSM**: most long-tail clouds don't offer managed HSM; would require either an on-prem HSM accessed remotely or a different key-protection model

The architecture works on these clouds where capabilities are met. Where capabilities are missing (e.g., no managed HSM, no Object Lock), they're genuinely unsuitable — the abstraction doesn't paper over real gaps.

### 27.5 IaC for multi-cloud

CDK is AWS-specific. For multi-cloud, **Pulumi** is the appropriate tool:

- Same TypeScript codebase can target AWS, GCP, Azure, and Kubernetes
- Construct-level abstractions can be shared
- Operational tooling (preview, deploy, destroy) is consistent

If multi-cloud is ever a requirement, the migration is:

1. Translate the CDK stacks to Pulumi stacks (mostly mechanical)
2. Add provider-specific stacks for the new cloud
3. Configure deployment pipelines for each cloud target

The application code (Go services) doesn't change — that's the whole point of the abstraction layer.

### 27.6 Effort estimate for adding a cloud

Given the v1 architecture, adding GCP or Azure is a moderate effort, not a rewrite:

| Task | Estimated effort |
|---|---|
| Implement cloud-specific backends (ObjectStore, ObjectLock, ReplicatedKV, HSM) | 2–3 weeks |
| Pulumi infra for the new cloud | 1–2 weeks |
| Multi-cloud test matrix (chaos, integration) | 1–2 weeks |
| Documentation, runbooks, CI updates | 1 week |
| Total | ~5–8 weeks per additional cloud |

The bulk of the work is implementation, not architecture. The shared test suite (§9.7) makes validation systematic rather than bespoke.

### 27.7 What this means for v1

Three small things v1 must do correctly to make multi-cloud cheap later:

1. **Build the cloud abstraction layer (§9) from the start.** Not optional, not future work. It's a v1 design choice.
2. **Use logical region names internally.** Don't hardcode `us-east-1` strings. Use `primary`, `dr-1`, `dr-2` and map at config edges.
3. **Use OpenTelemetry, not direct CloudWatch SDK calls.** OTEL is cloud-agnostic; CloudWatch ingests OTEL natively.

These are low-cost-now, high-value-later patterns. None of them complicates v1. All of them are good architecture independent of multi-cloud aspirations.

---

## 28. References

### Specifications

- `draft-ietf-plants-merkle-tree-certs` (currently -03)
- **WG GitHub repo**: `github.com/ietf-plants-wg/merkle-tree-certs` — bleeding-edge source between draft revisions; track here for in-flight changes and example code (`draft-ietf-plants-merkle-tree-certs.md`)
- `draft-ietf-tls-trust-anchor-ids`
- `RFC 9162` — Certificate Transparency v2
- `FIPS 204` — ML-DSA
- `tlog-tiles` — Transparency log tile serving format
- `RFC 5280` — X.509 Certificate and CRL Profile

**Spec tracking workflow**: For implementation work, pin to a specific published draft revision (`-03` at v0.6). When the WG GitHub `main` diverges from the latest published draft, that divergence is the next revision; review it for changes that will affect our implementation. A CI job pulls the GitHub `main` daily and diffs against our pinned version; significant changes file a Beads ticket automatically.

### Implementations

- `github.com/bwesterb/mtc` (BSD-3-Clause): reference impl by spec co-author. Read for design patterns; reimplement clean-room in Rust. Used as differential-testing oracle (cross-language).
- `qux-pqc` (BSD-3-Clause): FIPS-targeted PQ crypto in Rust (ML-DSA, ML-KEM, SLH-DSA). Primary PQ crypto dependency.
- RustCrypto crates (MIT/Apache-2.0): ECDSA, SHA-2, X25519, etc.
- `github.com/cloudflare/circl` (BSD-3-Clause): Go reference for PQ crypto patterns; not a runtime dependency.
- `github.com/digicert/mtc-bridge` (**AGPL-3.0**): read-only design reference; **cannot fork**.

### Rust ecosystem

- Tokio (MIT): async runtime
- `aws-sdk-rust` (Apache-2.0): AWS SDK
- `axum` (MIT): HTTP framework
- `clap` v4 (MIT/Apache-2.0): CLI
- `serde` (MIT/Apache-2.0): serialization
- `tracing` (MIT): structured logging + spans
- `eyre` + `color-eyre` (MIT/Apache-2.0): error reporting for binaries (colored panics, precise locations)
- `thiserror` (MIT/Apache-2.0): error type derivation for libraries
- `proptest` (MIT/Apache-2.0): property-based testing
- `arbitrary` (MIT/Apache-2.0): structured input generation (bridge between `proptest` and `cargo-fuzz`)
- `cargo-fuzz` + libFuzzer (MIT): unstructured input fuzzing
- `kani` (Apache-2.0/MIT): formal verification model checker
- `loom` (MIT): concurrency model checker
- `shuttle` (MIT): randomized concurrency testing
- `askama` (MIT/Apache-2.0): compile-time templates
- `qux-pqc` (BSD-3-Clause): FIPS-targeted PQ crypto

### Background reading

- Cloudflare blog: "Keeping the Internet fast and secure: introducing Merkle Tree Certificates"
- Google Security blog: "Cultivating a robust and efficient quantum-safe HTTPS"
- DigiCert blog: "Inside DigiCert's MTC Playground"
- IETF PLANTS mailing list archive

---

*End of document.*
