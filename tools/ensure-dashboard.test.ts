import { expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// `scripts/ensure-dashboard.sh` is tested at its one seam, the process: it runs in a scratch home
// against a stand-in for the supervisor, so nothing here can reach the live fleet.

type Options = {
  /** Each bot's supervisor status: running, restarting, stopped or failed. */
  bots?: Record<string, string>;
  /** The dashboard's supervisor status. */
  dashboard?: string;
  /** What `start dashboard` says when it is refused, if it is. */
  refused?: string;
};

function ensure(options: Options = {}) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-ensure-"));
  try {
    const calls = join(dir, "calls");
    writeFileSync(calls, "");
    const stub = (name: string, body: string) =>
      writeFileSync(join(dir, name), `#!/usr/bin/env bash\n${body}`, { mode: 0o755 });
    // A `systemctl` that refuses: a script that reached for it would be touching the real fleet.
    stub("systemctl", "echo 'systemctl is not for tests' >&2\nexit 1\n");

    const bots = options.bots ?? { First: "running", Second: "stopped" };
    const config = join(dir, ".config/svanbot");
    mkdirSync(config, { recursive: true });
    const status: Record<string, string> = { dashboard: options.dashboard ?? "stopped" };
    for (const [name, value] of Object.entries(bots)) {
      writeFileSync(join(config, `${name}.env`), `SVANBOT_NAME=${name}\n`);
      status[name] = value;
    }
    const env: Record<string, string> = {};
    for (const [name, value] of Object.entries(status)) env[`SVANBOT_TEST_STATUS_${name}`] = value;

    stub(
      "svanbot-bot",
      `printf 'svanbot-bot %s\\n' "$*" >> "$SVANBOT_TEST_CALLS"
if [[ $1 == status ]]; then
    var=SVANBOT_TEST_STATUS_$2
    echo "\${!var:-stopped}"
    [[ \${!var:-stopped} == running ]] && exit 0
    exit 3
fi
if [[ $1 == start && $2 == dashboard && -n "\${SVANBOT_TEST_REFUSED:-}" ]]; then
    echo "\${SVANBOT_TEST_REFUSED}" >&2
    exit 1
fi
exit 0
`,
    );

    const result = Bun.spawnSync(
      ["bash", join(import.meta.dir, "../scripts/ensure-dashboard.sh")],
      {
        env: {
          ...process.env,
          HOME: dir,
          PATH: `${dir}:${process.env.PATH}`,
          SVANBOT_BOT: join(dir, "svanbot-bot"),
          SVANBOT_TEST_CALLS: calls,
          SVANBOT_TEST_REFUSED: options.refused ?? "",
          ...env,
        },
        stdout: "pipe",
        stderr: "pipe",
      },
    );
    return {
      code: result.exitCode,
      out: `${result.stdout}${result.stderr}`,
      // What was asked of the supervisor, leaving out the questions about how a bot is doing.
      calls: readFileSync(calls, "utf8")
        .split("\n")
        .filter((line) => line && !line.startsWith("svanbot-bot status")),
      ran: existsSync(join(dir, ".local/state/svanbot/ensure-dashboard.ran")),
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("a bot runs and the dashboard does not: the dashboard is started", () => {
  const ran = ensure({ dashboard: "stopped" });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual(["svanbot-bot start dashboard"]);
  expect(ran.out).toContain("starting it");
});

test("a dashboard that runs, or is restarting under its supervisor, is left alone", () => {
  for (const dashboard of ["running", "restarting"]) {
    const ran = ensure({ dashboard });
    expect(ran.code).toBe(0);
    expect(ran.calls).toEqual([]);
    expect(ran.out).toBe("");
  }
});

test("a dashboard that failed, or was stopped by hand, is started again", () => {
  for (const dashboard of ["failed", "stopped"]) {
    expect(ensure({ dashboard }).calls).toEqual(["svanbot-bot start dashboard"]);
  }
});

test("a bot waiting to restart is still an active bot", () => {
  const ran = ensure({ bots: { First: "restarting", Second: "stopped" } });
  expect(ran.calls).toEqual(["svanbot-bot start dashboard"]);
});

test("with no bot running the dashboard is left as it is", () => {
  for (const dashboard of ["stopped", "running"]) {
    const ran = ensure({ bots: { First: "stopped", Second: "failed" }, dashboard });
    expect(ran.code).toBe(0);
    expect(ran.calls).toEqual([]);
  }
});

test("a start refused because the dashboard just came up is not an error", () => {
  // `update.sh` restarts the dashboard at the end of a rollout, and may start it a moment before.
  const raced = ensure({ refused: "svanbot-bot: dashboard is already running" });
  expect(raced.code).toBe(0);
  // Any other refusal is.
  const broken = ensure({ refused: "svanbot-bot: dashboard has no program to run" });
  expect(broken.code).toBe(1);
  expect(broken.out).toContain("has no program to run");
});

test("every run leaves a mark, so a mark gone stale says cron stopped running it", () => {
  expect(ensure({ dashboard: "running" }).ran).toBe(true);
  expect(ensure({ bots: { First: "stopped" } }).ran).toBe(true);
});
