import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function withSystemctl(
  check: (
    run: (script: string, locked?: boolean) => ReturnType<typeof Bun.spawnSync>,
    log: () => string,
  ) => void,
) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-fleet-"));
  try {
    const calls = join(dir, "calls");
    const config = join(dir, ".config/svanbot");
    mkdirSync(config, { recursive: true });
    for (const file of ["First.env", "Second.env", "invalid-name.env", "First.env.bak"]) {
      writeFileSync(join(config, file), "");
    }
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
      (script, locked) => {
        const lock = locked ? ["flock", join(dir, "update.lock")] : [];
        return Bun.spawnSync([...lock, "bash", join(import.meta.dir, "../scripts", script)], {
          env: {
            ...process.env,
            HOME: dir,
            PATH: `${dir}:${process.env.PATH}`,
            SVANBOT_LIB: dir,
            SVANBOT_TEST_CALLS: calls,
          },
          stderr: "pipe",
        });
      },
      () => readFileSync(calls, "utf8"),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("start brings up the configured bot units and dashboard", () => {
  withSystemctl((run, log) => {
    expect(run("start.sh").exitCode).toBe(0);
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

test("a release update in progress blocks both operations", () => {
  withSystemctl((run, log) => {
    for (const script of ["start.sh", "stop.sh"]) {
      const result = run(script, true);
      expect(result.exitCode).toBe(1);
      expect(result.stderr?.toString()).toContain("release update is already in progress");
    }
    expect(log()).toBe("");
  });
});
