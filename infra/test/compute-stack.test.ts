import { App } from "aws-cdk-lib";
import { Template } from "aws-cdk-lib/assertions";
import { MtcComputeStack } from "../lib/compute-stack";

function synthTemplate(): Template {
  const app = new App();
  const stack = new MtcComputeStack(app, "MtcComputeStack", {
    envName: "test",
  });
  return Template.fromStack(stack);
}

describe("MtcComputeStack", () => {
  test("synthesizes with no real resources yet (resource modeling is a later ticket)", () => {
    const template = synthTemplate();
    // Near-empty by design: the only entries CDK may add on its own are
    // synthesizer metadata (CDKMetadata / bootstrap version rule), never
    // billable compute resources.
    template.resourceCountIs("AWS::ECS::Service", 0);
    template.resourceCountIs("AWS::Lambda::Function", 0);
  });

  test("exposes the compute abstraction seam marker output", () => {
    const template = synthTemplate();
    template.hasOutput("ComputePurpose", {
      Value: "mtc-compute-test",
    });
  });

  test("template snapshot", () => {
    const template = synthTemplate();
    expect(template.toJSON()).toMatchSnapshot();
  });
});
