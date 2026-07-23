# Living Architecture Diagrams

Mermaid diagrams that render directly on GitHub — visual system context for agents and humans
without reading the full spec. Mandated by
[`docs/mtc-architecture-spec.md`](../mtc-architecture-spec.md) §23.6 ("Living architecture
diagrams").

**The spec is the source of truth; these diagrams are derived views.** Every page states the
spec section it renders and the conditions under which it must be updated. If a diagram and the
spec disagree, the spec wins — fix the diagram.

## Index

| Page | Renders spec section | Contents |
|---|---|---|
| [`system-overview.md`](system-overview.md) | §7 | Components and data flow: control plane, CA Service internals, Lambdas, HSM, storage |
| [`write-path-sequence.md`](write-path-sequence.md) | §11 | Issuance lifecycle sequence: intake → batch builder → tree updater → checkpoint signer → commit |
| [`multi-region-topology.md`](multi-region-topology.md) | §7, §8.3, §13 | Three-region active-passive topology, replication, lease/epoch failover state machine |
| [`data-model.md`](data-model.md) | §8 | S3 object layout and DynamoDB single-table coordination schema |

## Conventions

- **Mermaid only, no external assets** — diagrams must render on github.com with zero plugins.
- **One page per concern**, each with a "Source of truth" and "Update this page when" header.
- **Update in the same PR** as the change that invalidates a diagram (CI drift detection between
  diagrams and code is tracked separately under the foundation-infra epic).

## Validation

```bash
make diagrams-check      # validate Mermaid syntax in all docs/architecture pages
```

The checker (`scripts/check-diagrams.sh`) extracts every ` ```mermaid ` fence and parses it with
the same Mermaid engine GitHub uses. First run installs pinned tooling into
`scripts/diagrams-lint/node_modules` (network needed once); subsequent runs are offline.

```bash
make diagrams-check-selftest   # E2E smoke: checker must FAIL on a known-broken fixture
```
