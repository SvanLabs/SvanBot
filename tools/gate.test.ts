// The gate's tiers, tested at their one seam: a tier name in, the steps it runs out. The expected
// lists are the tiers as V11-03 sets them, not derived from the code under test.
import { expect, test } from "bun:test";
import { plan } from "./gate";

const names = (tier: string) => plan(tier).map((step) => step.name);

test("the commit tier runs the layering check and clippy", () => {
  expect(names("commit")).toEqual(["layering", "clippy"]);
});

test("the full tier adds the workspace tests and the tool tests", () => {
  expect(names("full")).toEqual(["layering", "clippy", "cargo-test", "tools-test"]);
});

test("an unknown tier is refused", () => {
  expect(() => plan("nightly")).toThrow(/unknown tier "nightly"/);
});

test("cargo steps build into target/dev, at low priority, on four jobs", () => {
  const cargoSteps = plan("full").filter((s) => s.name === "clippy" || s.name.startsWith("cargo"));
  for (const step of cargoSteps) {
    expect(step.env).toEqual({ CARGO_TARGET_DIR: "target/dev" });
    expect(step.argv.slice(0, 4)).toEqual(["nice", "-n", "15", "ionice"]);
    expect(step.argv).toContain("-j4");
  }
});

test("the layering step runs the checker with the Bun that runs the gate", () => {
  expect(plan("commit")[0]?.argv).toEqual([process.execPath, "tools/layering.ts"]);
});
