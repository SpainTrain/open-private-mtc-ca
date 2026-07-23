import { Construct } from "constructs";

/**
 * Properties for the {@link Compute} construct.
 */
export interface ComputeProps {
  /**
   * Logical environment name (e.g. "dev"). Used only for naming/tagging;
   * never implies a real AWS account.
   */
  readonly envName: string;
}

/**
 * Construct-level abstraction over the MTC CA compute platform
 * (spec section 4, "CDK TypeScript with abstraction in mind").
 *
 * This construct will eventually own the provider-specific compute
 * resources — the ECS Fargate service for the write path (CA service,
 * ACME, admin API) and the Lambdas for proof serving and event glue
 * (spec sections 5, 7). Keeping them behind this seam makes a future
 * Pulumi port a structural translation rather than a rewrite.
 *
 * Intentionally near-empty: real resource modeling is a separate ticket
 * (fnd-cdk-compute-stack).
 */
export class Compute extends Construct {
  /** Purpose marker surfaced by the owning stack for tests/demo. */
  public readonly purpose: string;

  constructor(scope: Construct, id: string, props: ComputeProps) {
    super(scope, id);
    this.purpose = `mtc-compute-${props.envName}`;
  }
}
