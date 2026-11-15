// The disk budget, tested at its one seam: a tree in, target/'s size against the budget out.
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
    line: "target/ holds 1.5 MiB of its 10.0 MiB budget",
  });
});

test("a target over the budget fails and says how to free it", () => {
  const root = tree({ "target/dev/a": 3 * MiB });
  expect(targetUse(root, 2 * MiB)).toEqual({
    over: true,
    line: "target/ holds 3.0 MiB, over its 2.0 MiB budget: run `cargo clean` in this clone",
  });
});

test("a hard-linked file counts once, as du counts it", () => {
  const root = tree({ "target/dev/deps/svanbot": 2 * MiB });
  linkSync(join(root, "target/dev/deps/svanbot"), join(root, "target/dev/svanbot"));
  expect(targetUse(root, 10 * MiB).line).toBe("target/ holds 2.0 MiB of its 10.0 MiB budget");
});

test("a clone that has not built yet holds nothing", () => {
  expect(targetUse(tree({ "src/lib.rs": 10 }), MiB)).toEqual({
    over: false,
    line: "target/ holds 0 B of its 1.0 MiB budget",
  });
});
