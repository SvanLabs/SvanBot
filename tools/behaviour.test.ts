// The behaviour trigger, tested at its one seam: given the paths a change touches, can it move a
// scoreboard line?
import { expect, test } from "bun:test";
import { join } from "node:path";
import { behaviourPaths } from "./behaviour";

test("a change in a pure layer can change how the bots play", () => {
  expect(behaviourPaths(["crates/strategy/src/policy.rs"])).toEqual([
    "crates/strategy/src/policy.rs",
  ]);
  expect(behaviourPaths(["crates/core/src/rules.rs", "crates/sim/src/deal.rs"])).toEqual([
    "crates/core/src/rules.rs",
    "crates/sim/src/deal.rs",
  ]);
});

test("a change that only wires the pure layers to the venue cannot", () => {
  expect(
    behaviourPaths([
      "crates/runtime/src/client.rs",
      "crates/apps/svanbot/src/main.rs",
      "tools/gate.ts",
      "README.md",
    ]),
  ).toEqual([]);
});

test("a path that only begins like a layer is not in it", () => {
  // `crates/core-web/` is not `crates/core/`: the prefix is the layer and its slash.
  expect(behaviourPaths(["crates/core-web/src/x.rs", "crates/sim.toml"])).toEqual([]);
});

test("a Windows path is read as its repo path", () => {
  expect(behaviourPaths(["crates\\learn\\src\\fit.rs"])).toEqual(["crates/learn/src/fit.rs"]);
});

test("the tool reads a diff one path a line, and answers with an exit code", async () => {
  // The workflow pipes `git diff --name-only` in, so a diff is never split into batches. A pure
  // path runs the Arena; an impure one alone does not; an empty diff is no change to play.
  const run = async (stdin: string) => {
    const child = Bun.spawn(["bun", "tools/behaviour.ts"], {
      cwd: join(import.meta.dir, ".."),
      stdin: new TextEncoder().encode(stdin),
      stdout: "pipe",
      stderr: "pipe",
    });
    return { code: await child.exited, out: await new Response(child.stdout).text() };
  };
  expect(await run("crates/learn/src/arena.rs\nREADME.md\n")).toEqual({
    code: 0,
    out: "crates/learn/src/arena.rs\n",
  });
  expect((await run("README.md\n")).code).toBe(1);
  expect((await run("\n")).code).toBe(1);
});
