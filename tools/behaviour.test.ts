// The behaviour trigger, tested at its one seam: given the paths a change touches, can it move a
// scoreboard line?
import { expect, test } from "bun:test";
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
