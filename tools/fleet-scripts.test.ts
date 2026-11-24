import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function withSystemctl(
  check: (
    run: (script: string, ...args: string[]) => ReturnType<typeof Bun.spawnSync>,
    log: () => string,
  ) => void,
) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-fleet-"));
  try {
    const calls = join(dir, "calls");
    writeFileSync(calls, "");
    writeFileSync(
      join(dir, "systemctl"),
      `#!/usr/bin/env bash
printf '%s\n' "$*" >> "$SVANBOT_TEST_CALLS"
if [[ $2 == list-units ]]; then
    printf '%s\n' 'svanbot@First.service loaded active running' \\
        'svanbot@Second.service loaded activating start' 'unrelated.service loaded active running'
fi
`,
      { mode: 0o755 },
    );
    check(
      (script, ...args) =>
        Bun.spawnSync(["bash", join(import.meta.dir, "../scripts", script), ...args], {
          env: {
            ...process.env,
            PATH: `${dir}:${process.env.PATH}`,
            SVANBOT_LIB: dir,
            SVANBOT_TEST_CALLS: calls,
          },
          stderr: "pipe",
        }),
      () => readFileSync(calls, "utf8"),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("start brings up the requested bot units and dashboard", () => {
  withSystemctl((run, log) => {
    expect(run("start.sh", "First", "Second").exitCode).toBe(0);
    expect(log()).toBe(
      "--user start svanbot@First.service svanbot@Second.service svanbot-dashboard.service\n",
    );
  });
});

test("stop brings down only the loaded bot units and dashboard", () => {
  withSystemctl((run, log) => {
    expect(run("stop.sh").exitCode).toBe(0);
    expect(log()).toContain(
      "--user stop svanbot@First.service svanbot@Second.service svanbot-dashboard.service\n",
    );
    expect(log()).not.toContain("unrelated.service");
  });
});
