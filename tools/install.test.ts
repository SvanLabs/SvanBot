// `deploy/install.sh`, run as the operator runs it, against a stand-in for `/proc/cpuinfo`: a
// bundle built for an instruction level the CPU does not have would fault the first time the bot
// reached an instruction it lacks, so the installer refuses it before anything is staged.
import { afterAll, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { type Files, pack } from "./release";

const root = mkdtempSync(join(tmpdir(), "svanbot-install-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

const INSTALL = join(import.meta.dir, "../deploy/install.sh");

/** The box's own CPU: every x86-64-v2 flag, and none of v3's. */
const V2 = "lm cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3 sse sse2";
const V3 = `${V2} avx avx2 bmi1 bmi2 f16c fma abm movbe xsave`;

/** A bundle that says it was built for `cpu`, or that says nothing when `cpu` is undefined. */
function bundle(dir: string, cpu: string | undefined): string {
  const files: Files = new Map([
    ["release/svanbot", Buffer.from("#!/bin/sh\n")],
    ["release/sha256", Buffer.from("0  svanbot\n")],
    ["release/version", Buffer.from("version\t11.1.0\nsha256\t0\n")],
    ["UPGRADE/svanbot-release", Buffer.from("#!/bin/sh\n")],
  ]);
  if (cpu !== undefined) files.set("UPGRADE/cpu", Buffer.from(`${cpu}\n`));
  const path = join(dir, "svanbot-11.1.0.tar.gz");
  writeFileSync(path, pack(files));
  return path;
}

function install(cpu: string | undefined, flags: string) {
  const dir = mkdtempSync(join(root, "box-"));
  const home = join(dir, "home");
  mkdirSync(home);
  const cpuinfo = join(dir, "cpuinfo");
  writeFileSync(cpuinfo, `processor\t: 0\nflags\t\t: ${flags}\n\nprocessor\t: 1\nflags\t\t: ${flags}\n`);
  const run = Bun.spawnSync(["sh", INSTALL, bundle(dir, cpu), join(home, "tree")], {
    env: { HOME: home, PATH: process.env.PATH ?? "/usr/bin:/bin", SVANBOT_CPUINFO: cpuinfo },
  });
  return {
    code: run.exitCode,
    stderr: run.stderr.toString(),
    staged: existsSync(join(home, "tree/staged/svanbot")),
  };
}

test("a bundle above the CPU's instruction level is refused, and nothing is staged", () => {
  const found = install("x86-64-v3", V2);
  expect(found.code).toBe(1);
  expect(found.stderr).toContain("x86-64-v3");
  expect(found.stderr).toContain("x86-64-v2");
  expect(found.staged).toBe(false);
});

test("a bundle at or below the CPU's level is staged", () => {
  expect(install("x86-64-v2", V2)).toMatchObject({ code: 0, staged: true });
  expect(install("x86-64-v2", V3)).toMatchObject({ code: 0, staged: true });
  expect(install("x86-64-v3", V3)).toMatchObject({ code: 0, staged: true });
});

test("a bundle that does not name its level is staged as baseline, the level every 64-bit CPU has", () => {
  expect(install(undefined, V2)).toMatchObject({ code: 0, staged: true });
});

test("a level the installer does not know is refused rather than guessed at", () => {
  const found = install("x86-64-v9", V3);
  expect(found.code).toBe(1);
  expect(found.staged).toBe(false);
});
