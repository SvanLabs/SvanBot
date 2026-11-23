// The one status the `main` ruleset requires is the `check` job. It passes only when the jobs it
// stands for did, so a merge that waits on it waits on all of them.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

type Job = { needs?: string[]; steps: [{ run: string }] };
const workflow = Bun.YAML.parse(
  readFileSync(new URL("../.github/workflows/check.yml", import.meta.url), "utf8"),
) as { jobs: Record<string, Job> & { check: Job } };

const GREEN = {
  EVENT: "pull_request",
  COMMIT: "success",
  TEST: "success",
  DENY: "success",
  ARENA: "success",
  PUBLICATION: "success",
};

/** Runs the `check` job's script with each job's result, and returns its exit code. */
function check(results: Record<string, string>): number {
  const run = Bun.spawnSync(["bash", "-c", workflow.jobs.check.steps[0].run], {
    env: { ...process.env, ...results },
  });
  return run.exitCode;
}

test("every job green is a passing check", () => {
  expect(check(GREEN)).toBe(0);
});

test("one tier that failed or never ran is a failing check", () => {
  for (const job of ["COMMIT", "TEST", "DENY", "ARENA", "PUBLICATION"]) {
    for (const result of ["failure", "cancelled", "skipped"]) {
      expect(check({ ...GREEN, [job]: result })).not.toBe(0);
    }
  }
});

test("on a push, the publication does not run and is not waited for", () => {
  expect(check({ ...GREEN, EVENT: "push", PUBLICATION: "skipped" })).toBe(0);
  expect(check({ ...GREEN, EVENT: "push", ARENA: "failure", PUBLICATION: "skipped" })).not.toBe(0);
});

test("the check needs every other job, so none can be added and left out of it", () => {
  const others = Object.keys(workflow.jobs).filter((id) => id !== "check");
  expect([...(workflow.jobs.check.needs ?? [])].sort()).toEqual(others.sort());
});
