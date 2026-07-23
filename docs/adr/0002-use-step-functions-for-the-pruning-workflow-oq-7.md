# ADR-0002: Use Step Functions for the pruning workflow (OQ-7)

- **Status**: Accepted
- **Date**: 2026-07-23
- **Spec sections**: §1 (non-goals: no production cloud spend), §5 (compute
  platform: pruning worker row), §9.2 (workflow orchestration excluded from the
  cloud abstraction), §15 (pruning and retention), §24 (Phase 7), §26 (OQ-7),
  §27.1 (workflows portability row)

## Context

Open question OQ-7 (§26) asks: Step Functions or a simple Lambda chain for the
pruning workflow, to be decided during the pruning epic. The workflow (§15.2)
is scheduled, multi-step, and correctness-sensitive: compute the prunable
range, sign and persist a pruning checkpoint, advance the replicated watermark,
wait for replication, then delete leaf objects whose S3 Object Lock
(Compliance-mode) retention has expired. It runs only on the primary
(lease-enforced), and a half-completed run — checkpoint committed but watermark
not advanced, or a deletion pass interrupted midway — must be resumable,
idempotent, and auditable. §5 already leans this way ("Pruning Worker: Lambda +
Step Functions — scheduled, multi-step"); OQ-7 stayed open pending one real
uncertainty: does Step Functions run in the zero-cost local world at all?

Forces in play:

- **Local-only constraint (§1 non-goals)**: no production cloud spend; all
  development runs against the pinned LocalStack **community** image
  (`localstack/localstack:4.14.0` in `deploy/local/docker-compose.yml` — the
  2026 CalVer images require an auth token). Whatever orchestrates pruning
  must execute on that image, for free, on a laptop.
- **Portability (§27)**: Step Functions is AWS-proprietary. §9.2 deliberately
  excludes workflow orchestration from the cloud abstraction — "abstract
  inside the service binary, not the deployment topology" — and §27.1 maps
  the Workflows row to GCP Workflows / Logic Apps / Argo / Temporal.
- **Failure semantics**: pruning is the one place the system deliberately
  deletes data; §15.1 requires pruning be "never silent". Orchestration state
  must be durable and inspectable, and the compliance Pruning log (§20.3) and
  pruning-failure runbook depend on knowing exactly where a run stopped.
- **Agent-driven testability (§23)**: agents validate work through
  deterministic, scriptable checks; the orchestrator must be drivable and
  assertable from a shell.

## Decision

We will orchestrate the pruning workflow with **AWS Step Functions** (a
Standard state machine), as §5 recommends. The state machine wires the
planner, commit protocol, and deletion executor as **thin Lambda task states**;
all domain logic (eligibility, checkpoint construction, watermark advance,
deletion rules) lives in the shared Rust crates those handlers invoke. The
Amazon States Language definition contains sequencing, retry policy, and
terminal states only — nothing that would need porting logic-wise to another
orchestrator.

The spike script [`scripts/spike-prune-stepfn.sh`](../../scripts/spike-prune-stepfn.sh)
proves the mechanism runs locally: it starts an ephemeral LocalStack community
container (the same pinned 4.14.0 image as the dev env), creates a trivial
pruning-shaped state machine (Plan → Choice → Wait → Succeed), executes it to
`SUCCEEDED`, and prints the per-state execution history. This honors the §1
local-only constraint end to end: Step Functions is available in the community
edition, no LocalStack account or auth token is needed, and no cloud resource
is ever created.

## Alternatives Considered

### Simple Lambda chain (Lambda A invokes B invokes C)

Rejected on failure semantics and on where the complexity ends up:

- **Retry/failure semantics**: synchronous invocation chains couple timeouts
  (the caller pays for the callee, compounding toward the 15-minute cap);
  asynchronous invocation gives only two coarse retries plus a DLQ. Neither
  leaves a durable record of *where* a multi-step run stopped, so resumability
  would require hand-rolling execution state in DynamoDB — rebuilding a small,
  bespoke, unaudited workflow engine on exactly the code path (deliberate data
  deletion) where §15.1 demands nothing be silent. Step Functions gives
  per-state `Retry` (backoff, max attempts) and `Catch` declaratively, plus a
  queryable execution history down to individual state transitions (verified in
  the spike via `get-execution-history`).
- **Complexity is conserved, not avoided**: the chain needs less CDK but more
  bespoke Rust orchestration code, which then needs its own unit and
  failure-injection tests. The state machine moves that complexity into
  declarative config that `cdk synth` assertions can verify.
- **Operational visibility**: with a chain, workflow status only exists if we
  build it. With Step Functions, `list-executions` / `describe-execution` /
  `get-execution-history` exist on day one — feeding the runbook, the §20.3
  compliance narrative, and the workflow metrics ticket.
- **Local debuggability** — the chain's real advantage — mostly survives the
  decision: each step remains an independently invocable Lambda handler over
  shared crates, so a developer (or agent) can still run any single step with a
  JSON payload without the state machine in the loop.
- **Cost does not discriminate**: in the LocalStack-only world both options
  are exactly $0. Modeled production cost (kept honest per §5): a daily
  pruning run of under ten state transitions is a few hundred transitions per
  month — fractions of a cent at Standard-workflow pricing, against Lambda
  invocation costs that are rounding error either way. Cost is a wash; the
  decisive factors are failure semantics and visibility.

### Hand-rolled orchestration inside the CA service (no Lambda, no Step Functions)

Run pruning as a scheduled loop inside the long-lived Fargate write-path task.
Rejected: §5 explicitly assigns the pruning worker to Lambda ("scheduled,
multi-step") rather than the CA service, keeping a bulk S3 delete/list workload
out of the latency-sensitive lease/batch loop; and it has the same hand-rolled
resumability problem as the Lambda chain while adding failure coupling to the
write path. It would be the most portable option, but §9.2 draws the
portability line at the service binary, not the deployment topology — the
shared pruning crates are the portable artifact, and they stay portable under
Step Functions too.

## Consequences

### Positive

- Retry, backoff, catch, and terminal failure states are declarative ASL
  config, assertable in `cdk synth` tests instead of hand-written Rust.
- Every pruning run has a durable, queryable execution record (status +
  per-state history), directly serving the "never silent" requirement (§15.1),
  the pruning-failure runbook, and the §20.3 compliance Pruning log.
- Agent-friendly validation: `awslocal stepfunctions start-execution` /
  `describe-execution` make E2E checks deterministic shell one-liners.
- Zero-cost constraint verified, not assumed: the spike runs on the pinned
  community image with no account, token, or cloud resources.
- Downstream tickets proceed as scoped (`prune-stepfn-workflow`,
  `prune-cdk-workflow` already name Step Functions).

### Negative

- **AWS-proprietary orchestration**: the ASL definition and its CDK constructs
  must be rewritten per cloud target (GCP Workflows / Logic Apps / Argo /
  Temporal, §27.1). Accepted and bounded: §9.2 excludes workflow orchestration
  from the abstraction on purpose, and the definition is kept logic-free so
  only a thin declarative file ports.
- **Emulator-coverage watch item**: local fidelity depends on LocalStack's
  community Step Functions emulator. The workflow must stay within
  well-emulated ASL (Task, Choice, Wait, Retry/Catch — verified for the basic
  states by the spike); exotic features (e.g. Distributed Map) are avoided.
- **Dev-env surface grows**: `deploy/local/docker-compose.yml` currently
  enables only `s3,dynamodb`; `prune-stepfn-workflow` must add `stepfunctions`
  and `lambda` to `SERVICES` (plus the Docker socket mount the LocalStack
  Lambda provider needs to spawn runtime containers). Until then the spike
  runs its own ephemeral container on port 4567.
- One more moving part between "invoke the step" and "the step ran" when
  debugging the full workflow (mitigated by directly invocable step handlers).
