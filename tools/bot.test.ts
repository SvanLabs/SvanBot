// The supervisor, `scripts/bot.sh`, tested against a stand-in program in a scratch home: what it
// starts, what it restarts, when it gives up, and what `status` says at each point.
import { afterEach, expect, setDefaultTimeout, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SCRIPT = join(import.meta.dir, "../scripts/bot.sh");
const homes: string[] = [];
// A crash loop and a learner step are watched for real, in real time.
setDefaultTimeout(20_000);

afterEach(() => {
  for (const home of homes.splice(0)) {
    // A test that failed half-way may leave a supervisor behind: stop everything it started.
    for (const name of ["First", "dashboard", "learn"]) bot(home, "stop", name);
    rmSync(home, { recursive: true, force: true });
  }
});

/**
 * A scratch home with one bot, `First`, whose env file carries `env`. The stand-in for
 * `current/svanbot` logs each start, its arguments and its environment to `runs`, then does what
 * `FAKE` says: `serve` runs until it is sent TERM, and `exit:N` exits with N at once.
 */
function home(env: Record<string, string> = { FAKE: "serve" }): string {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-bot-"));
  homes.push(dir);
  // A `systemctl` that refuses, first on the PATH: nothing under test may reach the live fleet.
  mkdirSync(join(dir, "bin"), { recursive: true });
  writeFileSync(join(dir, "bin/systemctl"), "#!/usr/bin/env bash\nexit 1\n", { mode: 0o755 });
  mkdirSync(join(dir, ".config/svanbot"), { recursive: true });
  writeFileSync(
    join(dir, ".config/svanbot/First.env"),
    `${Object.entries(env)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n")}\n# a comment\n\nSVANBOT_NAME="First"\n`,
  );
  mkdirSync(join(dir, "lib/current"), { recursive: true });
  writeFileSync(
    join(dir, "lib/current/svanbot"),
    `#!/usr/bin/env bash
printf 'start %s name=%s store=%s ulimit=%s stray=%s\\n' "$*" "\${SVANBOT_NAME:-}" "\${SVANBOT_STORE:-}" "$(ulimit -v)" "\${SVANBOT_ROSTER:-}" >> "$FAKE_RUNS"
# A learner step ends at once.
[[ \${1:-} == learn ]] && exit 0
case \${FAKE:-serve} in
    serve) trap 'echo term >> "$FAKE_RUNS"; exit 0' TERM; while :; do sleep 0.05; done ;;
    exit:*) exit "\${FAKE#exit:}" ;;
esac
`,
    { mode: 0o755 },
  );
  writeFileSync(join(dir, "runs"), "");
  return dir;
}

function bot(dir: string, ...args: string[]): { code: number; out: string } {
  return under(dir, [], ...args);
}

/** `bot`, run under `wrapper` (for example a lock) and with `extra` in the caller's environment. */
function under(dir: string, wrapper: string[], ...args: string[]): { code: number; out: string } {
  const result = Bun.spawnSync([...wrapper, "bash", SCRIPT, ...args], {
    env: {
      SVANBOT_ROSTER: "the caller's own roster",
      PATH: `${dir}/bin:${process.env.PATH ?? ""}`,
      HOME: dir,
      SVANBOT_LIB: join(dir, "lib"),
      SVANBOT_STATE: join(dir, "state"),
      SVANBOT_RESTART_SEC: "0",
      SVANBOT_LEARN_EVERY: "0",
      FAKE_RUNS: join(dir, "runs"),
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  return { code: result.exitCode ?? -1, out: `${result.stdout}${result.stderr}`.trim() };
}

function runs(dir: string): string[] {
  return readFileSync(join(dir, "runs"), "utf8").split("\n").filter(Boolean);
}

/** Polls `status` until it says `word`, for at most five seconds. */
function until(dir: string, name: string, word: string): string {
  let said = "";
  for (let i = 0; i < 100; i++) {
    said = bot(dir, "status", name).out;
    if (said === word) return said;
    Bun.sleepSync(50);
  }
  return said;
}

test("start runs the bot from current with its env file and store, and stop ends it", () => {
  const dir = home();
  expect(bot(dir, "status", "First").out).toBe("stopped");
  expect(bot(dir, "start", "First").code).toBe(0);
  expect(bot(dir, "status", "First")).toEqual({ code: 0, out: "running" });
  expect(runs(dir)).toEqual([
    `start  name=First store=${dir}/.local/share/svanbot/First.db ulimit=1048576 stray=`,
  ]);
  expect(bot(dir, "stop", "First").code).toBe(0);
  // Stop waits for the program to exit: TERM reached it, and nothing is restarted after it.
  expect(runs(dir).slice(1)).toEqual(["term"]);
  expect(bot(dir, "status", "First")).toEqual({ code: 3, out: "stopped" });
  expect(existsSync(join(dir, "state/First.log"))).toBe(true);
});

test("a bot that is running is not started twice", () => {
  const dir = home();
  expect(bot(dir, "start", "First").code).toBe(0);
  const again = bot(dir, "start", "First");
  expect(again.code).toBe(1);
  expect(again.out).toContain("First is already running");
  expect(runs(dir)).toHaveLength(1);
});

test("a crash is restarted, and five starts in five minutes leave the bot down as failed", () => {
  const dir = home({ FAKE: "exit:3" });
  expect(bot(dir, "start", "First").code).toBe(0);
  expect(until(dir, "First", "failed")).toBe("failed");
  expect(runs(dir)).toHaveLength(5);
  expect(readFileSync(join(dir, "state/First.log"), "utf8")).toContain(
    "5 starts in 300 s: First stays down",
  );
});

test("exit 0 is a stop, and exit 1 or 2 is a failure no restart can fix", () => {
  for (const [code, word] of [
    ["0", "stopped"],
    ["1", "failed"],
    ["2", "failed"],
  ] as const) {
    const dir = home({ FAKE: `exit:${code}` });
    expect(bot(dir, "start", "First").code).toBe(0);
    expect(until(dir, "First", word)).toBe(word);
    expect(runs(dir)).toHaveLength(1);
  }
});

test("a failed bot can be started again", () => {
  const dir = home({ FAKE: "exit:1" });
  bot(dir, "start", "First");
  expect(until(dir, "First", "failed")).toBe("failed");
  expect(bot(dir, "start", "First").code).toBe(0);
  expect(until(dir, "First", "failed")).toBe("failed");
  expect(runs(dir)).toHaveLength(2);
});

test("the dashboard and the learner run the program's own commands", () => {
  const dir = home();
  expect(bot(dir, "start", "dashboard").code).toBe(0);
  expect(until(dir, "dashboard", "running")).toBe("running");
  bot(dir, "stop", "dashboard");
  expect(runs(dir)[0]).toStartWith("start dashboard ");
  // The learner is one step a run, and a step that ends well is followed by the next one.
  writeFileSync(join(dir, "runs"), "");
  const learn = bot(dir, "start", "learn");
  expect(learn.code).toBe(0);
  for (let i = 0; i < 100 && runs(dir).length < 2; i++) Bun.sleepSync(50);
  bot(dir, "stop", "learn");
  expect(runs(dir).filter((line) => line.startsWith("start learn ")).length).toBeGreaterThan(1);
});

test("a name that is neither a bot nor a service is refused before anything runs", () => {
  const dir = home();
  for (const args of [["start", "../etc"], ["start"], ["status", "a b"], ["frobnicate", "First"]]) {
    expect(bot(dir, ...args).code).toBe(2);
  }
  expect(bot(dir, "start", "Nobody").code).toBe(2);
  expect(runs(dir)).toEqual([]);
});

test("a bot starts from its env file alone: the caller's SVANBOT_ variables do not reach it", () => {
  const dir = home();
  bot(dir, "start", "First");
  // The caller exported a roster; the bot's file names none, so the bot has none.
  expect(runs(dir)[0]).toEndWith(" stray=");
});

test("a supervisor does not hold a lock its starter held", () => {
  const dir = home();
  const lock = join(dir, "update.lock");
  // start.sh takes the update lock, and every supervisor it starts outlives it.
  expect(under(dir, ["flock", lock], "start", "First").code).toBe(0);
  expect(Bun.spawnSync(["flock", "-n", lock, "true"]).exitCode).toBe(0);
});

test("a bot another process already plays is not started twice: two connections are one too many", () => {
  const dir = home();
  // The bot as systemd (or a hand) runs it, outside any supervisor: the same program, the same name.
  mkdirSync(join(dir, "lib/11.0.0"), { recursive: true });
  writeFileSync(join(dir, "lib/11.0.0/svanbot"), "#!/usr/bin/env bash\nsleep 30\n", {
    mode: 0o755,
  });
  const other = Bun.spawn([join(dir, "lib/11.0.0/svanbot")], {
    env: { ...process.env, SVANBOT_NAME: "First" },
  });
  try {
    Bun.sleepSync(100);
    const started = bot(dir, "start", "First");
    expect(started.code).toBe(1);
    expect(started.out).toContain(`First already plays as pid ${other.pid}`);
    expect(runs(dir)).toEqual([]);
    // Another bot's process is not this one.
    other.kill();
  } finally {
    other.kill();
  }
});
