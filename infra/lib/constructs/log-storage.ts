import { Construct } from "constructs";

/**
 * Properties for the {@link LogStorage} construct.
 */
export interface LogStorageProps {
  /**
   * Logical environment name (e.g. "dev"). Used only for naming/tagging;
   * never implies a real AWS account.
   */
  readonly envName: string;
}

/**
 * Construct-level abstraction over the MTC log's durable storage
 * (spec section 4, "CDK TypeScript with abstraction in mind").
 *
 * This construct will eventually own the provider-specific storage
 * resources — the S3 bucket holding tiles/entries/checkpoints and the
 * DynamoDB coordination table (spec sections 4, 7, 8). Keeping them behind
 * this seam makes a future Pulumi port a structural translation rather
 * than a rewrite.
 *
 * Intentionally near-empty: real resource modeling is a separate ticket
 * (fnd-cdk-storage-stack).
 */
export class LogStorage extends Construct {
  /** Purpose marker surfaced by the owning stack for tests/demo. */
  public readonly purpose: string;

  constructor(scope: Construct, id: string, props: LogStorageProps) {
    super(scope, id);
    this.purpose = `mtc-log-storage-${props.envName}`;
  }
}
