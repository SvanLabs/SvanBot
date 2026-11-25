import { expect, test } from "bun:test";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// `scripts/auto-update.sh` is tested at its one seam, the process: it runs in a scratch home against
// stand-ins for every tool it calls, so nothing here can reach the live fleet. A bot "running a
// build" is a copy of `sleep` named `lib/<version>/svanbot`, which is what /proc/<pid>/exe names.
const SLEEP = Bun.which("sleep") ?? "/usr/bin/sleep";

type Options = {
  /** The newest release GitHub names. */
  latest?: string;
  /** Each bot's running build, or null when it is down. */
  bots?: Record<string, string | null>;
  /** The builds installed under lib/. */
  installed?: string[];
  /** What the stand-in for update.sh exits with. */
  update?: number;
  /** Files to put in the state directory before the run. */
  state?: Record<string, string>;
  /** Another auto-update holds the lock while this one starts. */
  locked?: boolean;
};

function autoUpdate(options: Options = {}) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-auto-"));
  const children: ReturnType<typeof Bun.spawn>[] = [];
  try {
    const calls = join(dir, "calls");
    writeFileSync(calls, "");
    const stub = (name: string, body: string, where = dir) =>
      writeFileSync(join(where, name), `#!/usr/bin/env bash\n${body}`, { mode: 0o755 });

    // A `systemctl` that refuses, and a `svanbot-release` that refuses and says so: a script that
    // reached for either would be touching the real fleet.
    stub("systemctl", "echo 'systemctl is not for tests' >&2\nexit 1\n");
    stub(
      "svanbot-release",
      `printf 'REFUSED svanbot-release %s\\n' "$*" >> "$SVANBOT_TEST_CALLS"\nexit 1\n`,
    );

    const bots = options.bots ?? { First: "11.1.6", Second: "11.1.6" };
    const config = join(dir, ".config/svanbot");
    mkdirSync(config, { recursive: true });
    for (const name of Object.keys(bots)) {
      writeFileSync(join(config, `${name}.env`), `SVANBOT_NAME=${name}\n`);
    }
    stub("app-token.sh", "echo app-token\n", config);

    const lib = join(dir, "lib");
    // A build a bot runs is installed by definition, so its directory exists whatever `installed` says.
    const running = Object.values(bots).filter((version): version is string => Boolean(version));
    for (const version of new Set([...(options.installed ?? ["11.1.6", "11.1.7"]), ...running])) {
      mkdirSync(join(lib, version), { recursive: true });
      copyFileSync(SLEEP, join(lib, version, "svanbot"));
      chmodSync(join(lib, version, "svanbot"), 0o755);
    }

    const pids: Record<string, string> = {};
    for (const [name, version] of Object.entries(bots)) {
      if (!version) continue;
      const child = Bun.spawn([join(lib, version, "svanbot"), "60"], {
        stdout: "ignore",
        stderr: "ignore",
      });
      children.push(child);
      pids[`SVANBOT_TEST_PID_${name}`] = String(child.pid);
    }

    stub(
      "svanbot-bot",
      `printf 'svanbot-bot %s\\n' "$*" >> "$SVANBOT_TEST_CALLS"
if [[ $1 == pid ]]; then
    var=SVANBOT_TEST_PID_$2
    [[ -n \${!var:-} ]] && { echo "\${!var}"; exit 0; }
    exit 3
fi
exit 0
`,
    );
    stub(
      "gh",
      `printf 'gh %s token=%s\\n' "$*" "\${GH_TOKEN:-}" >> "$SVANBOT_TEST_CALLS"
[[ $1 == release && $2 == view ]] && echo "v$SVANBOT_TEST_LATEST"
exit 0
`,
    );
    stub(
      "update",
      `printf 'update.sh %s token=%s\\n' "$*" "\${SVANBOT_GH_TOKEN:-}" >> "$SVANBOT_TEST_CALLS"
# The bots update.sh restarts live for days: the App's token must not be in their environment.
[[ -n \${GH_TOKEN:-} ]] && echo 'LEAKED GH_TOKEN to update.sh' >> "$SVANBOT_TEST_CALLS"
# A bot's supervisor, started from here, would inherit the lock and hold it for good.
[[ -e /proc/$$/fd/8 ]] && echo 'LEAKED the lock descriptor to update.sh' >> "$SVANBOT_TEST_CALLS"
exit "\${SVANBOT_TEST_UPDATE_EXIT:-0}"
`,
    );

    const state = join(dir, ".local/state/svanbot");
    mkdirSync(state, { recursive: true });
    for (const [file, text] of Object.entries(options.state ?? {})) {
      writeFileSync(join(state, file), text);
    }

    const held = options.locked ? ["flock", join(state, "auto-update.lock")] : [];
    const result = Bun.spawnSync(
      [...held, "bash", join(import.meta.dir, "../scripts/auto-update.sh")],
      {
        env: {
          ...process.env,
          GH_TOKEN: "",
          HOME: dir,
          PATH: `${dir}:${process.env.PATH}`,
          SVANBOT_LIB: lib,
          SVANBOT_BOT: join(dir, "svanbot-bot"),
          SVANBOT_UPDATE: join(dir, "update"),
          SVANBOT_TEST_CALLS: calls,
          SVANBOT_TEST_LATEST: options.latest ?? "11.1.7",
          SVANBOT_TEST_UPDATE_EXIT: String(options.update ?? 0),
          ...pids,
        },
        stdout: "pipe",
        stderr: "pipe",
      },
    );
    const left: Record<string, string> = {};
    for (const file of readdirSync(state)) left[file] = readFileSync(join(state, file), "utf8");
    return {
      code: result.exitCode,
      out: `${result.stdout}${result.stderr}`,
      calls: readFileSync(calls, "utf8")
        .split("\n")
        .filter((line) => line && !line.startsWith("svanbot-bot pid")),
      // Read before the scratch home goes: what the script left in its state directory.
      state: (file: string) => left[file] ?? null,
    };
  } finally {
    for (const child of children) child.kill();
    rmSync(dir, { recursive: true, force: true });
  }
}

test("a running bot on an older build gets the newest release from update.sh, as the App", () => {
  const ran = autoUpdate();
  expect(ran.code).toBe(0);
  // The token is the App's, from its helper: the operator's own `gh` login is never used.
  expect(ran.calls).toEqual([
    "gh release view --repo SvanLabs/SvanBot --json tagName --jq .tagName token=app-token",
    "update.sh --running-only 11.1.7 token=app-token",
  ]);
});

const LOOKUP =
  "gh release view --repo SvanLabs/SvanBot --json tagName --jq .tagName token=app-token";

test("when every running bot already runs the newest release, nothing is rolled out and nothing is said", () => {
  const ran = autoUpdate({ bots: { First: "11.1.7", Second: "11.1.7" } });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual([LOOKUP]);
  expect(ran.out).toBe("");
});

test("bots that are down are left alone: a fleet that is all down asks nobody anything", () => {
  const down = autoUpdate({ bots: { First: null, Second: null } });
  expect(down.code).toBe(0);
  expect(down.calls).toEqual([]);
  // One running bot on an older build is enough: update.sh skips the down one (--running-only).
  const mixed = autoUpdate({ bots: { First: "11.1.6", Second: null } });
  expect(mixed.calls).toEqual([LOOKUP, "update.sh --running-only 11.1.7 token=app-token"]);
});

test("a bot on a newer build than the release is not rolled back to it", () => {
  const ran = autoUpdate({ bots: { First: "11.1.8", Second: "11.1.7" } });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual([LOOKUP]);
});

const NOW = () => Math.floor(Date.now() / 1000);
const UPDATE = "update.sh --running-only 11.1.7 token=app-token";

test("a release that installed but did not take is left alone for six hours, then tried again", () => {
  const failed = autoUpdate({ update: 1 });
  expect(failed.code).toBe(1);
  expect(failed.state("auto-update.failed")).toMatch(/^11\.1\.7 \d+\n$/);
  expect(failed.out).toContain("11.1.7 did not take");

  const soon = autoUpdate({ state: { "auto-update.failed": `11.1.7 ${NOW() - 3600}\n` } });
  expect(soon.code).toBe(0);
  expect(soon.calls).toEqual([LOOKUP]);
  expect(soon.out).toContain("did not take earlier");

  const later = autoUpdate({ state: { "auto-update.failed": `11.1.7 ${NOW() - 7 * 3600}\n` } });
  expect(later.calls).toEqual([LOOKUP, UPDATE]);
});

test("a newer release is tried at once, and a rollout that works forgets the failure", () => {
  const newer = autoUpdate({ state: { "auto-update.failed": `11.1.6 ${NOW() - 60}\n` } });
  expect(newer.calls).toEqual([LOOKUP, UPDATE]);
  expect(newer.state("auto-update.failed")).toBeNull();

  const healed = autoUpdate({ state: { "auto-update.failed": `11.1.7 ${NOW() - 7 * 3600}\n` } });
  expect(healed.code).toBe(0);
  expect(healed.state("auto-update.failed")).toBeNull();
});

test("a release that could not be installed is tried again next time, and no failure is kept", () => {
  const ran = autoUpdate({ installed: ["11.1.6"], update: 1 });
  expect(ran.code).toBe(1);
  expect(ran.out).toContain("could not be installed");
  expect(ran.state("auto-update.failed")).toBeNull();
});

test("a release name that is not a version is refused before anything is rolled out", () => {
  const ran = autoUpdate({ latest: "nightly" });
  expect(ran.code).toBe(1);
  expect(ran.calls).toEqual([LOOKUP]);
  expect(ran.out).toContain("not a version");
});

test("a paused auto-update does nothing and says so", () => {
  const ran = autoUpdate({ state: { "auto-update.pause": "" } });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual([]);
  expect(ran.out).toContain("paused");
});

test("a second auto-update while one runs does nothing", () => {
  const ran = autoUpdate({ locked: true });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual([]);
  expect(ran.out).toContain("already running");
});
