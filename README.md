# MTC-CA — Merkle Tree Certificate CA

A reference implementation of a Merkle Tree Certificate (MTC) Certificate
Authority, per [`draft-ietf-plants-merkle-tree-certs`](https://datatracker.ietf.org/doc/draft-ietf-plants-merkle-tree-certs/).
The design targets a multi-region, active-passive CA with an append-only Merkle
Tree log, HSM-backed signing, ACME issuance, and the signatureless-certificate
optimization that keeps post-quantum certificates small.

The authoritative design is [`docs/mtc-architecture-spec.md`](docs/mtc-architecture-spec.md).
Everything in this repository is organized around implementing that spec.

## What this is (and is not)

This is a **reference blueprint** for evaluating the MTC draft and exploring what
tier-zero PKI demands — not a product. The non-goals below are foundational; they
shape every decision in the repo (see spec §1):

- **No production deployment.** This is not a commercially supported product and
  is not intended for production use by the author.
- **Zero cloud spend.** Production costs are *modeled* to keep the design honest
  (e.g. CloudHSM run rates), but all development runs against the zero-cost local
  simulation environment (LocalStack + SoftHSM2) described in spec §18. No real
  AWS account is ever targeted; the CDK app in `infra/` is synth-only.
- **Not public Web PKI.** Single trust boundary; the CA and its relying parties
  are assumed to share one. Not a Chrome-trusted CA.
- **Manual failover in v1.** Strong tooling for a human-driven failover, not
  automated failover.

Everything is meant to be runnable, demoable, and debuggable on a laptop.

## Repository layout

| Path          | Contents                                                                              |
|---------------|---------------------------------------------------------------------------------------|
| `crates/`     | Rust workspace — CA services and libraries (spec §4, §9.3)                             |
| `infra/`      | TypeScript AWS CDK app — synth-only, LocalStack-targeted, never a real deploy (§4)     |
| `api/`        | OpenAPI specification and code generated from it (server stubs, clients) (§17.2)       |
| `docs/`       | Architecture spec, dev-environment guide, ADRs, and design notes                      |
| `scripts/`    | Dev and ops shell scripts that `make` targets delegate to (§18.8)                     |
| `tests/e2e/`  | End-to-end tests driving `mtcctl` against the demo environment (§19.1)                 |
| `mk/`         | Makefile fragments; each adds targets without touching the root `Makefile` (see below) |

The Rust/TypeScript split is deliberate: `infra/` (CDK) and `crates/` (services)
never import types from each other. The deployment edge is the only boundary, and
config crosses it via SSM Parameter Store + env vars (spec §4).

## Quickstart

The toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml); with
[rustup](https://rustup.rs) installed, the pinned toolchain is fetched
automatically on first `cargo` invocation.

```bash
make help        # list every available dev workflow (self-documenting)
cargo test       # build and run the Rust workspace test suite
```

`make help` is the single entry point for the developer workflows in spec §18 —
the 60-second demo, fixtures, time travel, REPLs, and the rest — as those targets
are implemented. See [`docs/dev-environment.md`](docs/dev-environment.md) for the
target catalog and conventions, and [`CONTRIBUTING.md`](CONTRIBUTING.md) for how
changes are structured and reviewed.

## License

Licensed under the Apache License, Version 2.0. See [`LICENSE`](LICENSE).
