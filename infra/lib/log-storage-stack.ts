import { CfnOutput, Stack, StackProps } from "aws-cdk-lib";
import { Construct } from "constructs";
import { LogStorage } from "./constructs/log-storage";

/**
 * Properties for {@link MtcLogStorageStack}.
 */
export interface MtcLogStorageStackProps extends StackProps {
  /** Logical environment name (e.g. "dev"). */
  readonly envName: string;
}

/**
 * Stack owning the MTC log's durable storage: S3 (tiles, entries,
 * checkpoints, revocations) and DynamoDB (coordination state), per spec
 * sections 4 and 7.
 *
 * Split from {@link MtcComputeStack} deliberately (spec section 4) so
 * provider-specific resources stay behind construct-level abstractions,
 * enabling a future Pulumi port.
 *
 * Synth-only: this stack is never deployed to a real AWS account.
 */
export class MtcLogStorageStack extends Stack {
  constructor(scope: Construct, id: string, props: MtcLogStorageStackProps) {
    super(scope, id, props);

    const storage = new LogStorage(this, "LogStorage", {
      envName: props.envName,
    });

    new CfnOutput(this, "LogStoragePurpose", {
      description: "Marker output for the log storage abstraction seam",
      value: storage.purpose,
    });
  }
}
