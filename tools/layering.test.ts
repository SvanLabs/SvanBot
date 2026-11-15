// The layering rule of ADR 0003, tested at its one seam: given a workspace directory, which
// violations does it have? Each test writes a small Cargo workspace to a temporary directory.
import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { checkWorkspace } from "./layering";

type Member = {
  /** Path to the crate's directory, relative to the workspace root. */
  dir: string;
  /** Extra manifest text: `[dependencies]` and the other dependency tables. */
  manifest?: string;
  /** Source files, relative to the crate directory. */
  files?: Record<string, string>;
};

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function workspace(members: Member[], rootExtra = ""): string {
  const root = mkdtempSync(join(tmpdir(), "layering-"));
  roots.push(root);
  const list = members.map((m) => JSON.stringify(m.dir)).join(", ");
  write(root, "Cargo.toml", `[workspace]\nresolver = "3"\nmembers = [${list}]\n${rootExtra}`);
  for (const m of members) {
    const name = m.dir.split("/").at(-1);
    const manifest = `[package]\nname = "w-${name}"\nversion = "0.0.0"\nedition = "2024"\n`;
    write(root, `${m.dir}/Cargo.toml`, manifest + (m.manifest ?? ""));
    write(root, `${m.dir}/src/lib.rs`, "");
    for (const [path, text] of Object.entries(m.files ?? {})) write(root, `${m.dir}/${path}`, text);
  }
  return root;
}

function write(root: string, path: string, text: string) {
  const full = join(root, path);
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, text);
}

const path = (to: string) => `{ path = "../${to}" }`;

test("a dependency on a higher layer is an upward edge", () => {
  const root = workspace([
    { dir: "crates/core", manifest: `[dependencies]\nw-runtime = ${path("runtime")}\n` },
    { dir: "crates/runtime" },
  ]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-core", rule: "upward", detail: "w-runtime (runtime) is above core" },
  ]);
});

test("edges go down to any lower layer, and no app depends on another app", () => {
  const root = workspace([
    { dir: "crates/core" },
    { dir: "crates/learn", manifest: `[dependencies]\nw-core = ${path("core")}\n` },
    {
      dir: "crates/apps/bot",
      manifest: `[dependencies]\nw-learn = ${path("../learn")}\nw-release = ${path("release")}\n`,
    },
    { dir: "crates/apps/release", manifest: `[dependencies]\nw-core = ${path("../core")}\n` },
  ]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-bot", rule: "upward", detail: "w-release (apps) is another app" },
  ]);
});

test("a crate outside crates/<layer>/ has no layer", () => {
  const root = workspace([{ dir: "crates/core" }, { dir: "crates/util" }, { dir: "xtask" }]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-util", rule: "unplaced", detail: "crates/util is not under crates/<layer>/" },
    { crate: "w-xtask", rule: "unplaced", detail: "xtask is not under crates/<layer>/" },
  ]);
});

test("a pure layer takes no third-party crate outside its tests", () => {
  const root = workspace([
    {
      dir: "crates/core",
      manifest: `[dependencies]\ntokio = "1"\n[dev-dependencies]\nproptest = "1"\n[build-dependencies]\ncc = "1"\n`,
    },
    { dir: "crates/runtime", manifest: `[dependencies]\ntokio = "1"\nrusqlite = "0.37"\n` },
    { dir: "crates/apps/bot", manifest: `[dependencies]\naxum = "0.8"\n` },
  ]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-core", rule: "third-party", detail: "tokio in a pure layer (core)" },
    { crate: "w-core", rule: "third-party", detail: "cc (build) in a pure layer (core)" },
  ]);
});

test("a third-party crate on the workspace's pure allowlist may sit in a pure layer", () => {
  const allowlist = `[workspace.metadata.layering.pure-third-party]\nfixedbitset = "pure bit sets"\n`;
  const root = workspace(
    [{ dir: "crates/sim", manifest: `[dependencies]\nfixedbitset = "0.5"\nrand = "0.9"\n` }],
    allowlist,
  );
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-sim", rule: "third-party", detail: "rand in a pure layer (sim)" },
  ]);
});

test("library code in a pure layer never turns off the clippy bans on I/O", () => {
  const optOut = "#![allow(clippy::disallowed_methods, clippy::disallowed_types)]\n";
  const root = workspace([
    {
      dir: "crates/core",
      files: {
        "src/lib.rs": `#![allow(unused)]\n${optOut}`,
        "src/hand/mod.rs": "#[expect(\n    clippy::style,\n    reason = \"x\"\n)]\nfn f() {}\n",
        "tests/golden.rs": optOut,
      },
    },
    { dir: "crates/runtime", files: { "src/lib.rs": optOut } },
  ]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-core", rule: "io-opt-out", detail: "src/hand/mod.rs turns off clippy::style" },
    { crate: "w-core", rule: "io-opt-out", detail: "src/lib.rs turns off clippy::disallowed_methods" },
    { crate: "w-core", rule: "io-opt-out", detail: "src/lib.rs turns off clippy::disallowed_types" },
  ]);
});

test("a path dependency outside the workspace counts as third-party", () => {
  const outside = mkdtempSync(join(tmpdir(), "layering-outside-"));
  roots.push(outside);
  write(outside, "Cargo.toml", `[package]\nname = "vendored"\nversion = "0.0.0"\nedition = "2024"\n`);
  write(outside, "src/lib.rs", "");
  const root = workspace([
    { dir: "crates/strategy", manifest: `[dependencies]\nvendored = { path = "${outside}" }\n` },
  ]);
  expect(checkWorkspace(root)).toEqual([
    { crate: "w-strategy", rule: "third-party", detail: "vendored in a pure layer (strategy)" },
  ]);
});

test("this workspace keeps the rule", () => {
  expect(checkWorkspace(join(import.meta.dir, ".."))).toEqual([]);
});

test("the command lists each violation and fails", () => {
  const root = workspace([
    { dir: "crates/core", manifest: `[dependencies]\nw-runtime = ${path("runtime")}\n` },
    { dir: "crates/runtime" },
  ]);
  const run = Bun.spawnSync(["bun", join(import.meta.dir, "layering.ts"), root]);
  expect(run.exitCode).toBe(1);
  expect(run.stdout.toString()).toBe("w-core: upward: w-runtime (runtime) is above core\n");
});
