import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

function runGuard(locked: boolean) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-guard-"));
  try {
    const calls = join(dir, "calls");
    writeFileSync(calls, "");
    const stub = (name: string, body: string) =>
      writeFileSync(join(dir, name), `#!/bin/bash\n${body}\n`, { mode: 0o700 });
    stub("date", "echo 2026-11-25");
    stub("mkdir", "exit 0");
    stub("flock", locked ? "exit 1" : "exit 0");
    stub("nice", 'shift 2; exec "$@"');
    stub("ionice", 'shift; exec "$@"');
    stub("svanbot", 'printf "%s\\n" "$*" >> "$GUARD_TEST_CALLS"');
    stub("systemctl", 'echo systemctl >> "$GUARD_TEST_CALLS"; exit 1');
    const result = Bun.spawnSync(["/bin/bash", "scripts/guard.sh"], {
      env: {
        HOME: dir,
        PATH: dir,
        SVANBOT_STATE: dir,
        GUARD_TEST_CALLS: calls,
      },
    });
    return {
      exit: result.exitCode,
      calls: readFileSync(calls, "utf8"),
      log: result.stdout.toString(),
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("the cron guard runs the policy guard once without touching supervisors", () => {
  const result = runGuard(false);
  expect(result.exit).toBe(0);
  expect(result.calls).toBe("play guard\n");
  expect(result.log).toContain("2026-11-25");
});

test("another run holding the lock skips the policy guard", () => {
  const result = runGuard(true);
  expect(result.exit).toBe(0);
  expect(result.calls).toBe("");
  expect(result.log).toContain("already running");
});
