# infra/ — AWS CDK app for the MTC CA

TypeScript AWS CDK app modeling the infrastructure for the Merkle Tree
Certificate CA described in `docs/mtc-architecture-spec.md`.

## Policy: synth-only. No real AWS account is ever targeted.

This project is a reference blueprint and exploration of the MTC draft
(spec §1 non-goals). Accordingly:

- **`cdk synth` and unit tests are the only supported workflows.**
  `cdk deploy`, `cdk bootstrap`, and `cdk diff` against a real AWS
  account are out of policy and are never run.
- **No AWS credentials are required or used.** The app is deliberately
  environment-agnostic (no `env: { account, region }` in `bin/`), so
  synth never resolves a real account.
- **All runtime development happens locally** against LocalStack
  (S3, DynamoDB) and SoftHSM2 (PKCS#11), per spec §18 — zero cloud
  spend. The production costs in spec §5 (Fargate, CloudHSM) are
  _modeled_ to keep the design honest; they are never incurred.

## Layout

| Path                       | Purpose                                                                                                                                                                           |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bin/mtc-infra.ts`         | CDK app entry point; instantiates both stacks                                                                                                                                     |
| `lib/log-storage-stack.ts` | `MtcLogStorageStack` — will own S3 (tiles/entries/checkpoints) + DynamoDB (coordination) per spec §4, §7                                                                          |
| `lib/compute-stack.ts`     | `MtcComputeStack` — will own ECS Fargate (write path) + Lambda (read/event glue) per spec §5, §7                                                                                  |
| `lib/constructs/`          | Construct-level abstractions the stacks compose. Provider-specific resources live behind these seams so a future Pulumi port is a structural translation, not a rewrite (spec §4) |
| `test/`                    | `aws-cdk-lib/assertions` Template tests + snapshots per stack                                                                                                                     |

The storage/compute stack split is deliberate (spec §4, "CDK TypeScript
with abstraction in mind"). Both stacks are currently near-empty
skeletons; real resource modeling arrives in later tickets
(`fnd-cdk-storage-stack`, `fnd-cdk-compute-stack`).

## Toolchain

Node is pinned to `22.13.0` (`.nvmrc`, `engines`, and `volta` in
`package.json`). TypeScript runs with `strict: true`; eslint
(typescript-eslint, type-checked) and prettier are configured.

## Commands

```bash
cd infra
npm ci            # install pinned dependencies
npm run build     # tsc --strict type-check
npm run lint      # eslint
npm run fmt:check # prettier
npm test          # jest: CDK assertions + snapshot tests
npx cdk synth     # render CloudFormation for both stacks to stdout
```

`npx cdk synth` synthesizes both `MtcLogStorageStack` and
`MtcComputeStack`, writing their CloudFormation templates under
`cdk.out/` (git-ignored). To print a single stack's template to stdout:

```bash
npx cdk synth MtcLogStorageStack
npx cdk synth MtcComputeStack
```
