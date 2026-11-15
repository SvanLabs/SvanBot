// The gate's tiers, tested at their one seam: a tier name in, the steps it runs out. The expected
// lists are the tiers as V11-03 sets them, not derived from the code under test.
import { expect, test } from "bun:test";
import { plan } from "./gate";

const names = (tier: string) => plan(tier).map((step) => step.name);
const step = (tier: string, name: string) => plan(tier).find((s) => s.name === name);

test("the commit tier runs the fast checks first and clippy last", () => {
  expect(names("commit")).toEqual([
    "fmt",
    "biome",
    "file-size",
    "secrets",
    "provenance",
    "layering",
    "clippy",
  ]);
});

test("the full tier adds the tests and the type check, then reports the disk budget", () => {
  expect(names("full")).toEqual([
    "fmt",
    "biome",
    "file-size",
    "secrets",
    "provenance",
    "layering",
    "clippy",
    "nextest",
    "doc-test",
    "tsc",
    "tools-test",
    "disk",
  ]);
  expect(step("full", "disk")?.argv).toEqual([process.execPath, "tools/disk.ts"]);
});

test("the test tier is the full tier's tests alone, so CI can run it beside the commit tier", () => {
  expect(names("test")).toEqual(["nextest", "doc-test", "tsc", "tools-test"]);
});

test("the ci tier runs cargo-deny against the lockfile", () => {
  expect(plan("ci")).toEqual([
    { name: "deny", argv: ["cargo", "deny", "--locked", "check"], env: {} },
  ]);
});

test("an unknown tier is refused, naming the ones there are", () => {
  expect(() => plan("nightly")).toThrow('unknown tier "nightly": use commit, full, test or ci');
});

test("cargo builds into target/dev, at low priority, on four jobs", () => {
  const lowPriority = ["nice", "-n", "15", "ionice", "-c3", "cargo"];
  expect(step("full", "clippy")?.argv).toEqual([
    ...lowPriority,
    ...["clippy", "--workspace", "--all-targets", "-j4", "--", "-D", "warnings"],
  ]);
  expect(step("full", "doc-test")?.argv).toEqual([
    ...lowPriority,
    "test",
    "--workspace",
    "--doc",
    "-j4",
  ]);
  // nextest's own -j is its test threads, so the build jobs take their long flag.
  expect(step("full", "nextest")?.argv).toEqual([
    ...lowPriority,
    ...[
      "nextest",
      "run",
      "--workspace",
      "--no-tests=fail",
      "--build-jobs",
      "4",
      "--test-threads",
      "4",
    ],
  ]);
  for (const name of ["clippy", "nextest", "doc-test"]) {
    expect(step("full", name)?.env).toEqual({ CARGO_TARGET_DIR: "target/dev" });
  }
});

test("the tool checks run with the Bun that runs the gate", () => {
  expect(step("commit", "file-size")?.argv).toEqual([process.execPath, "tools/file-size.ts"]);
  expect(step("commit", "secrets")?.argv).toEqual([process.execPath, "tools/secrets.ts"]);
  expect(step("commit", "provenance")?.argv).toEqual([process.execPath, "tools/provenance.ts"]);
  expect(step("commit", "layering")?.argv).toEqual([process.execPath, "tools/layering.ts"]);
  expect(step("full", "tools-test")?.argv).toEqual([process.execPath, "test", "tools"]);
});

test("fmt, biome and tsc check without building or writing", () => {
  const checks = ["fmt", "biome", "tsc"].map((name) => step("full", name));
  expect(checks).toEqual([
    { name: "fmt", argv: ["cargo", "fmt", "--all", "--check"], env: {} },
    { name: "biome", argv: ["biome", "check", "tools"], env: {} },
    { name: "tsc", argv: ["tools/node_modules/.bin/tsc", "-p", "tools"], env: {} },
  ]);
});
