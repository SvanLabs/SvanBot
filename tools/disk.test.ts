// The disk budget, tested at its one seam: a tree in, all target/ directories against the budget out.
import { afterEach, expect, test } from "bun:test";
import { linkSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { targetUse } from "./disk";

const MiB = 2 ** 20;
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function tree(files: Record<string, number>): string {
  const root = mkdtempSync(join(tmpdir(), "disk-"));
  roots.push(root);
  for (const [path, bytes] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), Buffer.alloc(bytes));
  }
  return root;
}

test("a target under the budget passes and says how much it holds", () => {
  const root = tree({ "target/dev/a": MiB, "target/dev/deps/b": MiB / 2 });
  expect(targetUse(root, 10 * MiB)).toEqual({
    over: false,
    line: "target/ directories hold 1.5 MiB of this clone's 10.0 MiB budget",
  });
});

test("a target over the budget fails and says how to free it", () => {
  const root = tree({ "target/dev/a": 3 * MiB });
  expect(targetUse(root, 2 * MiB)).toEqual({
    over: true,
    line:
      "target/ directories hold 3.0 MiB, over this clone's 2.0 MiB budget: remove build output from every target/ directory in this clone\n" +
      "  3.0 MiB target",
  });
});

test("nested target directories contribute to the clone-wide budget", () => {
  const root = tree({
    "target/dev/a": MiB,
    "crates/apps/svanbot/web/target/release/b": MiB,
  });
  expect(targetUse(root, 1.5 * MiB)).toEqual({
    over: true,
    // Equal sizes are named in path order.
    line:
      "target/ directories hold 2.0 MiB, over this clone's 1.5 MiB budget: remove build output from every target/ directory in this clone\n" +
      "  1.0 MiB crates/apps/svanbot/web/target\n" +
      "  1.0 MiB target",
  });
});

test("an over-budget clone names each target directory, largest first", () => {
  const root = tree({
    "target/dev/a": MiB,
    ".claude/worktrees/old/target/dev/b": 2 * MiB,
  });
  expect(targetUse(root, 2 * MiB).line).toBe(
    "target/ directories hold 3.0 MiB, over this clone's 2.0 MiB budget: remove build output from every target/ directory in this clone\n" +
      "  2.0 MiB .claude/worktrees/old/target\n" +
      "  1.0 MiB target",
  );
});

test("git's objects and node_modules are not searched for target directories", () => {
  const root = tree({
    ".git/target/a": MiB,
    "web/node_modules/pkg/target/b": MiB,
  });
  expect(targetUse(root, MiB).line).toBe(
    "target/ directories hold 0 B of this clone's 1.0 MiB budget",
  );
});

test("a hard-linked file counts once, as du counts it", () => {
  const root = tree({ "target/dev/deps/svanbot": 2 * MiB });
  linkSync(join(root, "target/dev/deps/svanbot"), join(root, "target/dev/svanbot"));
  expect(targetUse(root, 10 * MiB).line).toBe(
    "target/ directories hold 2.0 MiB of this clone's 10.0 MiB budget",
  );
});

test("a hard-linked file across target directories counts once", () => {
  const root = tree({ "target/dev/a": 2 * MiB });
  const nested = join(root, "crates/apps/svanbot/web/target/release/a");
  mkdirSync(join(nested, ".."), { recursive: true });
  linkSync(join(root, "target/dev/a"), nested);
  expect(targetUse(root, 2 * MiB)).toEqual({
    over: false,
    line: "target/ directories hold 2.0 MiB of this clone's 2.0 MiB budget",
  });
});

test("a clone that has not built yet holds nothing", () => {
  expect(targetUse(tree({ "src/lib.rs": 10 }), MiB)).toEqual({
    over: false,
    line: "target/ directories hold 0 B of this clone's 1.0 MiB budget",
  });
});
