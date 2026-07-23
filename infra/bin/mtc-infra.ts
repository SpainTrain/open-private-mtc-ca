#!/usr/bin/env node
/**
 * CDK app entry point for the MTC CA infrastructure.
 *
 * Synth-only by policy (spec section 1 non-goals): this app exists to be
 * `cdk synth`-ed and unit-tested. It is intentionally environment-agnostic
 * (no account/region `env`) and is never deployed to a real AWS account.
 */
import { App } from "aws-cdk-lib";
import { MtcComputeStack } from "../lib/compute-stack";
import { MtcLogStorageStack } from "../lib/log-storage-stack";

const app = new App();

const envName = "dev";

new MtcLogStorageStack(app, "MtcLogStorageStack", {
  envName,
  description:
    "MTC CA log storage (S3 tiles/entries/checkpoints, DynamoDB coordination) - synth-only",
});

new MtcComputeStack(app, "MtcComputeStack", {
  envName,
  description:
    "MTC CA compute (ECS Fargate write path, Lambda read/event glue) - synth-only",
});
