// Exercise the publisher GitHub will run. Only the remote PR and comment calls are simulated.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const workflow = Bun.YAML.parse(
  readFileSync(new URL("../.github/workflows/check.yml", import.meta.url), "utf8"),
) as { jobs: { publication: { steps: [{ run: string }] } } };

function publish(scenario: string, required = "yes", artifact = "1") {
  const probe = `
import json, os, sys
from unittest.mock import patch
sha = os.environ['HEAD_SHA']
state = {'head': sha}
writes = []
def gh(args, **kwargs):
    if '--paginate' in args:
        return json.dumps([[{'id': 7, 'user': {'login': 'github-actions[bot]'}, 'body': '<!-- svanbot-arena-publication -->\\nold'}]])
    if '--jq' in args: return state['head']
    writes.append([args[3], args[-1]])
    if os.environ['SCENARIO'] == 'push': state['head'] = 'b' * 40
    return '{}'
with patch('subprocess.check_output', gh):
    try: exec(sys.stdin.read(), {}); status = 0
    except SystemExit as e: status = e.code
print(json.dumps({'status': status, 'writes': writes}))`;
  const result = Bun.spawnSync(["python3", "-c", probe], {
    stdin: new TextEncoder().encode(workflow.jobs.publication.steps[0].run),
    env: {
      ...process.env,
      SCENARIO: scenario,
      RESULT: "success",
      REQUIRED: required,
      PR: "7",
      HEAD_SHA: "a".repeat(40),
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_SERVER_URL: "https://github.com",
      GITHUB_RUN_ID: "2",
      ARTIFACT: `https://github.com/owner/repo/actions/runs/2/artifacts/${artifact}`,
      ROWS: '[{"opponent":"nit","deals":100,"ci95":5,"mbb_per_100":-10}]',
    },
  });
  if (result.exitCode !== 0) throw Error(result.stderr.toString());
  return JSON.parse(result.stdout.toString());
}

test("a losing run updates the scoreboard rather than failing the check", () => {
  const { status, writes } = publish("current");
  expect(status).toBe(0);
  expect(writes).toEqual([["PATCH", expect.stringContaining("| nit | -10 | 5 | 100 |")]]);
});

test("a push during publication cannot earn a passing check", () => {
  expect(publish("push").status).toBe("Superseded PR head");
});

test("a missing artifact cannot earn a passing check", () => {
  expect(publish("current", "yes", "")).toEqual({ status: "Missing run artifact", writes: [] });
});

test("a non-behaviour head replaces a prior scoreboard without an artifact", () => {
  const { status, writes } = publish("current", "no", "");
  expect(status).toBe(0);
  expect(writes).toEqual([
    ["PATCH", expect.stringContaining(`No Arena run required at \`${"a".repeat(40)}\``)],
  ]);
});
