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

/**
 * Runs `scripts/update.sh` with `args` against stand-ins for `svanbot-release`, `systemctl`, `gh`
 * and the installer, in a scratch home with bots `First` and `Second`, `Second` the canary. Each
 * stand-in logs what it was asked. `rollback` names a bot whose swap the stand-in rolls back.
 */
function update(
  args: string[],
  options: { installed?: boolean; rollback?: string; dashboard?: boolean } = {},
) {
  const dir = mkdtempSync(join(tmpdir(), "svanbot-update-"));
  try {
    const calls = join(dir, "calls");
    writeFileSync(calls, "");
    const config = join(dir, ".config/svanbot");
    mkdirSync(config, { recursive: true });
    writeFileSync(join(config, "First.env"), "SVANBOT_NAME=First\n");
    writeFileSync(join(config, "Second.env"), "SVANBOT_NAME=Second\nSVANBOT_CANARY=Second\n");
    const lib = join(dir, "lib");
    mkdirSync(join(lib, options.installed ? "11.1.7" : "11.1.0"), { recursive: true });
    const stub = (name: string, body: string) =>
      writeFileSync(join(dir, name), `#!/usr/bin/env bash\n${body}`, { mode: 0o755 });
    stub(
      "svanbot-release",
      `printf 'svanbot-release name=%s canary=%s %s\\n' "\${SVANBOT_NAME:-}" "\${SVANBOT_CANARY:-}" "$*" >> "$SVANBOT_TEST_CALLS"
if [[ $1 == update ]]; then
    bot=\${SVANBOT_NAME:-$SVANBOT_CANARY}
    [[ $2 == --fleet ]] || bot=$SVANBOT_CANARY
    if [[ $bot == "\${SVANBOT_TEST_ROLLBACK:-}" ]]; then
        echo "svanbot-release: \${@: -1} crash-looped on $bot; rolled back to 11.1.0"
    else
        echo "svanbot-release: $bot now runs \${@: -1}"
    fi
fi
`,
    );
    stub(
      "systemctl",
      `printf 'systemctl %s\\n' "$*" >> "$SVANBOT_TEST_CALLS"
[[ $2 == show ]] && echo 0
[[ $2 == is-active ]] && exit "\${SVANBOT_TEST_DASHBOARD:-3}"
exit 0
`,
    );
    stub(
      "gh",
      `printf 'gh %s\\n' "$*" >> "$SVANBOT_TEST_CALLS"
while (($#)); do [[ $1 == --dir ]] && touch "$2/svanbot-11.1.7-x86-64-v2.tar.gz"; shift; done
`,
    );
    stub("installer", `printf 'install.sh %s\\n' "\${1##*/}" >> "$SVANBOT_TEST_CALLS"\n`);
    const result = Bun.spawnSync(["bash", join(import.meta.dir, "../scripts/update.sh"), ...args], {
      env: {
        ...process.env,
        HOME: dir,
        PATH: `${dir}:${process.env.PATH}`,
        SVANBOT_LIB: lib,
        SVANBOT_INSTALLER: join(dir, "installer"),
        SVANBOT_TEST_CALLS: calls,
        SVANBOT_TEST_ROLLBACK: options.rollback ?? "",
        SVANBOT_TEST_DASHBOARD: options.dashboard ? "0" : "3",
      },
      stdout: "pipe",
      stderr: "pipe",
    });
    return {
      code: result.exitCode,
      out: `${result.stdout}${result.stderr}`,
      calls: readFileSync(calls, "utf8")
        .split("\n")
        .filter((line) => line && !line.startsWith("systemctl --user show")),
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("update installs a new release, swaps the canary, then each other bot, one at a time", () => {
  const ran = update(["v11.1.7"], { dashboard: true });
  expect(ran.code).toBe(0);
  // The download goes to a fresh directory of the script's own; the install, to the tree's stage.
  const downloadedTo = ran.calls[0]?.split("--dir ")[1] ?? "";
  const installedFrom = ran.calls[2]?.split("install ")[1] ?? "";
  expect(ran.calls).toEqual([
    `gh release download v11.1.7 --repo SvanLabs/SvanBot --dir ${downloadedTo}`,
    "install.sh svanbot-11.1.7-x86-64-v2.tar.gz",
    `svanbot-release name= canary= install ${installedFrom}`,
    "svanbot-release name=Second canary=Second update 11.1.7",
    "svanbot-release name=First canary= update --fleet 11.1.7",
    "systemctl --user is-active --quiet svanbot-dashboard.service",
    "systemctl --user restart svanbot-dashboard.service",
  ]);
  expect(ran.calls[2]).toEndWith("/lib/staged");
  expect(ran.out).toContain("Every bot runs 11.1.7");
});

test("an installed release is not downloaded again, and --canary-only stops at the canary", () => {
  const ran = update(["--canary-only", "11.1.7"], { installed: true });
  expect(ran.code).toBe(0);
  expect(ran.calls).toEqual(["svanbot-release name=Second canary=Second update 11.1.7"]);
  expect(ran.out).toContain("--fleet");
});

test("a swap that is rolled back stops the rollout there and says so", () => {
  const canary = update(["11.1.7"], { installed: true, rollback: "Second" });
  expect(canary.code).toBe(1);
  expect(canary.calls).toEqual(["svanbot-release name=Second canary=Second update 11.1.7"]);
  expect(canary.out).toContain("Second did not take 11.1.7");
  const fleet = update(["11.1.7"], { installed: true, rollback: "First" });
  expect(fleet.code).toBe(1);
  expect(fleet.out).toContain("First did not take 11.1.7");
});

test("a version that is not a version is refused before anything runs", () => {
  for (const args of [[], ["latest"], ["11.1"], ["11.1.7", "11.1.8"], ["--fleet", "11.1.7"]]) {
    const ran = update(args, { installed: true });
    expect(ran.code).toBe(2);
    expect(ran.calls).toEqual([]);
  }
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
