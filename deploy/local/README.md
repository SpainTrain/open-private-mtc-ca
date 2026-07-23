# Local dev environment: LocalStack + SoftHSM2

Zero-cost local simulation of the MTC CA's cloud dependencies
(spec §18.1, §9.6). No real AWS account is ever targeted (§1 non-goals).

| Service    | Emulates                                       | Spec       |
| ---------- | ---------------------------------------------- | ---------- |
| LocalStack | S3 (versioning + Object Lock), DynamoDB        | §8.1, §8.2 |
| SoftHSM2   | CloudHSM via PKCS#11 (ECDSA P-256 signing key) | §4, §14    |

## Quick start

From the repo root:

```bash
make dev-env-up      # start + wait for healthy (cold start < ~60s, §18.1)
make dev-env-smoke   # verify bucket/table/token provisioning
make dev-env-reset   # tear down and wipe all state (clean slate)
```

Without make:

```bash
docker compose -f deploy/local/docker-compose.yml up -d --build --wait
./scripts/dev-env-smoke.sh
./scripts/dev-env-reset.sh
```

## What gets provisioned

On startup, `localstack/init/ready.d/01-init-mtc.sh` runs inside LocalStack:

- **S3 bucket `mtc-log-local`** — versioning enabled, Object Lock enabled with a
  Compliance-mode default retention of 1 day (true append-only, §8 "S3
  invariants"; short retention because local state is disposable).
- **DynamoDB table `mtc-log-coordination`** — `PK` (S, HASH) + `SK` (S, RANGE),
  on-demand billing, matching the §8.2 single-table schema. Non-key attributes
  are schemaless.

The `softhsm` container entrypoint initializes a SoftHSM2 token and generates
an ECDSA P-256 keypair (`checkpoint-signing`, §14.1 v1 algorithm). Token state
lives in the `softhsm-tokens` volume, so it survives `docker compose restart`
and is destroyed by `dev-env-reset`.

## Endpoint / env-var contract

Machine-readable version: [`local.env`](./local.env) — source it with
`set -a; source deploy/local/local.env; set +a`.

Production config is resolved from SSM Parameter Store under
`/mtc/<env>/<component>/<key>` (the `fnd-cdk-ssm-config-handoff` contract);
in local dev the same values are supplied as env-var overrides via figment
layering (§4 config row). This table is the local-override side of that
contract.

### AWS endpoint + credentials

| Variable                | Local value             | Notes                                              |
| ----------------------- | ----------------------- | -------------------------------------------------- |
| `AWS_ENDPOINT_URL`      | `http://127.0.0.1:4566` | Honored natively by the Rust AWS SDK and AWS CLI   |
| `AWS_ACCESS_KEY_ID`     | `test`                  | LocalStack accepts any value; `test` is convention |
| `AWS_SECRET_ACCESS_KEY` | `test`                  | 〃                                                 |
| `AWS_DEFAULT_REGION`    | `us-east-1`             | Single-region locally (multi-region: §18.3, later) |
| `AWS_REGION`            | `us-east-1`             | Rust SDK reads `AWS_REGION`                        |

### MTC resources (spec §8)

| Variable                 | Local value            | SSM equivalent (production)               |
| ------------------------ | ---------------------- | ----------------------------------------- |
| `MTC_LOG_BUCKET`         | `mtc-log-local`        | `/mtc/<env>/storage/log-bucket-name`      |
| `MTC_COORDINATION_TABLE` | `mtc-log-coordination` | `/mtc/<env>/storage/coordination-table-name` |

### PKCS#11 / SoftHSM2 (spec §14)

| Variable                 | Local value                          | Notes                             |
| ------------------------ | ------------------------------------ | --------------------------------- |
| `MTC_PKCS11_MODULE_PATH` | `/usr/lib/softhsm/libsofthsm2.so`    | Path **inside** the softhsm container |
| `MTC_PKCS11_TOKEN_LABEL` | `mtc-dev`                            |                                   |
| `MTC_PKCS11_PIN`         | `1234`                               | Dev-only; never a real credential |
| `MTC_PKCS11_KEY_LABEL`   | `checkpoint-signing`                 | ECDSA P-256 (§14.1 v1)            |

To poke at the token:

```bash
docker compose -f deploy/local/docker-compose.yml exec softhsm \
  pkcs11-tool --module /usr/lib/softhsm/libsofthsm2.so --list-slots
```

**Host-side PKCS#11 (for the future Rust HSM crate):** a process on the host
cannot `dlopen` a module inside a container. When the cloud-abstraction epic
needs in-process PKCS#11, install SoftHSM2 locally (`apt install softhsm2
opensc` / `brew install softhsm`) and set `MTC_PKCS11_MODULE_PATH` to the host
path (Debian/Ubuntu: `/usr/lib/softhsm/libsofthsm2.so`, Homebrew:
`$(brew --prefix)/lib/softhsm/libsofthsm2.so`). The provisioning steps are the
same commands the container entrypoint runs (`deploy/local/softhsm/entrypoint.sh`).

## Known emulation gaps

- LocalStack (community) accepts and stores Object Lock configuration and the
  smoke test asserts it, but Compliance-mode *enforcement* fidelity is weaker
  than real S3 — do not treat local as proof of append-only enforcement.
- No persistence: LocalStack state is lost when the container stops (that is
  fine — `dev-env-up` re-provisions in seconds).
- Single region by default. A second LocalStack instance for exercising S3
  CRR / DynamoDB Global Tables replication is available as a Compose
  *override* — `docker-compose.replication-sim.yml` — layered on top of this
  file without changing it (`make replication-sim-up`); see
  `crates/dev-replicator` and `mk/replication-sim.mk`. The full three-region
  topology is the multi-region epic (§18.3, `dev-multiregion-harness`).
- SoftHSM2 is explicitly non-FIPS (§14.4).

## Out of scope here

`make demo` orchestration, CA service startup, sample ACME client, and the
admin UI belong to the local-dev-experience epic (§18.1). This directory only
provides the substrate: LocalStack + SoftHSM2 + provisioning.

Replication simulation between LocalStack instances
(`docker-compose.replication-sim.yml`, `crates/dev-replicator`) is documented
separately — see that crate's module docs and `mk/replication-sim.mk`. It
extends this directory (a second LocalStack instance) without modifying
`docker-compose.yml`, `local.env`, or any single-region behavior above.
