import { App } from "aws-cdk-lib";
import { Template } from "aws-cdk-lib/assertions";
import { MtcLogStorageStack } from "../lib/log-storage-stack";

function synthTemplate(): Template {
  const app = new App();
  const stack = new MtcLogStorageStack(app, "MtcLogStorageStack", {
    envName: "test",
  });
  return Template.fromStack(stack);
}

describe("MtcLogStorageStack", () => {
  test("synthesizes with no real resources yet (resource modeling is a later ticket)", () => {
    const template = synthTemplate();
    // Near-empty by design: the only entries CDK may add on its own are
    // synthesizer metadata (CDKMetadata / bootstrap version rule), never
    // billable storage resources.
    template.resourceCountIs("AWS::S3::Bucket", 0);
    template.resourceCountIs("AWS::DynamoDB::Table", 0);
    template.resourceCountIs("AWS::DynamoDB::GlobalTable", 0);
  });

  test("exposes the log storage abstraction seam marker output", () => {
    const template = synthTemplate();
    template.hasOutput("LogStoragePurpose", {
      Value: "mtc-log-storage-test",
    });
  });

  test("template snapshot", () => {
    const template = synthTemplate();
    expect(template.toJSON()).toMatchSnapshot();
  });
});
