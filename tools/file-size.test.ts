// The 500-line limit, tested at its one seam: given a tree, which Rust files are over it?
import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { oversized } from "./file-size";

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function tree(files: Record<string, number>): string {
  const root = mkdtempSync(join(tmpdir(), "file-size-"));
  roots.push(root);
  for (const [path, lines] of Object.entries(files)) {
    const full = join(root, path);
    mkdirSync(dirname(full), { recursive: true });
    writeFileSync(full, "//\n".repeat(lines));
  }
  return root;
}

test("a Rust file of 500 lines passes and one of 501 is reported", () => {
  const root = tree({ "crates/core/src/ok.rs": 500, "crates/core/src/big.rs": 501 });
  expect(oversized(root)).toEqual(["crates/core/src/big.rs has 501 lines; the limit is 500"]);
});

test("files outside crates/, other file types and build output are not counted", () => {
  const root = tree({
    "tools/long.ts": 900,
    "crates/core/README.md": 900,
    "crates/core/target/debug/gen.rs": 900,
    "crates/sim/tests/golden.rs": 501,
  });
  expect(oversized(root)).toEqual(["crates/sim/tests/golden.rs has 501 lines; the limit is 500"]);
});

test("a last line without a newline still counts", () => {
  const root = tree({});
  const full = join(root, "crates/core/src/edge.rs");
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, `${"//\n".repeat(500)}//`);
  expect(oversized(root)).toEqual(["crates/core/src/edge.rs has 501 lines; the limit is 500"]);
});
