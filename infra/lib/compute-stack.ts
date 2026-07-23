import { CfnOutput, Stack, StackProps } from "aws-cdk-lib";
import { Construct } from "constructs";
import { Compute } from "./constructs/compute";

/**
 * Properties for {@link MtcComputeStack}.
 */
export interface MtcComputeStackProps extends StackProps {
  /** Logical environment name (e.g. "dev"). */
  readonly envName: string;
}

/**
 * Stack owning the MTC CA compute platform: ECS Fargate for the write
 * path and Lambda for read serving and event glue, per spec sections 5
 * and 7.
 *
 * Split from {@link MtcLogStorageStack} deliberately (spec section 4) so
 * provider-specific resources stay behind construct-level abstractions,
 * enabling a future Pulumi port.
 *
 * Synth-only: this stack is never deployed to a real AWS account.
 */
export class MtcComputeStack extends Stack {
  constructor(scope: Construct, id: string, props: MtcComputeStackProps) {
    super(scope, id, props);

    const compute = new Compute(this, "Compute", {
      envName: props.envName,
    });

    new CfnOutput(this, "ComputePurpose", {
      description: "Marker output for the compute abstraction seam",
      value: compute.purpose,
    });
  }
}
