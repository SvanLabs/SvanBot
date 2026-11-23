// The release bundle's shape, tested at its one seam: a set of files in, the archive bytes out.
// The contract is not invented here — `release/` is what the updater's `install` verifies back
// (`crates/runtime/src/release.rs`), so these tests pack a bundle and read it with the system's own
// `tar`, which is what the installer will use.
import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import * as release from "./release";

const root = mkdtempSync(join(tmpdir(), "svanbot-release-tool-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

const PROGRAM = Buffer.from("#!/bin/sh\necho svanbot\n");
const UPDATER = Buffer.from("#!/bin/sh\necho svanbot-release\n");

/** A directory that looks like the clone: the two built programs, and what UPGRADE/ carries. */
function fakeClone(): string {
  const dir = mkdtempSync(join(root, "clone-"));
  mkdirSync(join(dir, "target/dev/release"), { recursive: true });
  mkdirSync(join(dir, "deploy"), { recursive: true });
  writeFileSync(join(dir, "target/dev/release", release.PROGRAM), PROGRAM);
  writeFileSync(join(dir, "target/dev/release", release.UPDATER), UPDATER);
  writeFileSync(join(dir, "deploy/install.sh"), "#!/bin/sh\n");
  writeFileSync(join(dir, "deploy/svanbot@.service"), "[Unit]\n");
  return dir;
}

test("the archive holds the staged release the updater verifies, and the files a fresh box needs", () => {
  const files = release.collect(fakeClone(), "2026.11.16");
  expect([...files.keys()].sort()).toEqual([
    "UPGRADE/cpu",
    "UPGRADE/install.sh",
    "UPGRADE/svanbot-release",
    "UPGRADE/svanbot@.service",
    "release/sha256",
    "release/svanbot",
    "release/version",
  ]);
});

test("the bundle says the instruction level it was built for, so the installer can check it", () => {
  expect(release.collect(fakeClone(), "2026.11.16", "x86-64-v2").get("UPGRADE/cpu")?.toString()).toBe(
    "x86-64-v2\n",
  );
  // The box is x86-64-v2, and that is what a release is built for unless told otherwise.
  expect(release.collect(fakeClone(), "2026.11.16").get("UPGRADE/cpu")?.toString()).toBe(
    `${release.CPU}\n`,
  );
  expect(release.CPU).toBe("x86-64-v2");
});

test("the updater travels outside the slot, so nothing execs it as a build", () => {
  const files = release.collect(fakeClone(), "2026.11.16");
  expect(files.get("UPGRADE/svanbot-release")?.equals(UPDATER)).toBe(true);
  expect(files.has("release/svanbot-release")).toBe(false);
});

test("a clone without the operator's tool is refused before anything is packed", () => {
  const dir = fakeClone();
  rmSync(join(dir, "target/dev/release", release.UPDATER));
  expect(() => release.collect(dir, "2026.11.16")).toThrow(
    /no .*target\/dev\/release\/svanbot-release: build the release first/,
  );
});

test("the manifest and the published checksum agree, and it is the program's own hash", () => {
  const files = release.collect(fakeClone(), "2026.11.16");
  const digest = release.sha256(PROGRAM);
  expect(files.get("release/sha256")?.toString()).toBe(`${digest}  svanbot\n`);
  const manifest = files.get("release/version")?.toString() ?? "";
  // Exactly the fields a reader acts on: `version` names the build, `sha256` is the bytes the
  // updater verifies against. `Release::at` reads those two and no others, so a third field here
  // would be a record with no reader — a promise the box does not keep.
  expect(manifest).toBe(`version\t2026.11.16\nsha256\t${digest}\n`);
});

test("the packed archive is a tar the system reads, with the program executable", () => {
  const archive = join(root, "bundle.tar.gz");
  writeFileSync(archive, release.pack(release.collect(fakeClone(), "2026.11.16")));
  const out = mkdtempSync(join(root, "unpacked-"));
  expect(Bun.spawnSync(["tar", "-xzf", archive, "-C", out]).exitCode).toBe(0);
  // The program's mode survives the round trip: the updater refuses a program it cannot exec, and
  // the unit execs it. `ls` reads the mode back as the system sees it.
  const listing = Bun.spawnSync(["ls", "-l", join(out, "release/svanbot")]).stdout.toString();
  expect(listing.slice(0, 10)).toStartWith("-rwxr-xr-x");
  expect(readFileSync(join(out, "release/sha256"), "utf8")).toContain(release.sha256(PROGRAM));
  expect(readFileSync(join(out, "UPGRADE/install.sh"), "utf8")).toBe("#!/bin/sh\n");
});

test("the members are sorted, and the header's checksum is the one tar checks", () => {
  const tar = release.tar(
    new Map([
      ["z", Buffer.from("last")],
      ["a", Buffer.from("first")],
    ]),
  );
  expect(tar.subarray(0, 1).toString()).toBe("a");
  // tar reads the second header block to find `z`; its own name field and checksum must verify.
  const second = tar.subarray(512 + 512, 1024 + 512);
  expect(second.subarray(0, 1).toString()).toBe("z");
  let sum = 0;
  for (let i = 0; i < 512; i += 1) {
    sum += i >= 148 && i < 156 ? 0x20 : (second.at(i) ?? 0);
  }
  const field = second.subarray(148, 156).toString();
  expect(Number.parseInt(field.slice(0, 6), 8)).toBe(sum);
});

test("the version comes from the workspace manifest unless the tag overrides it", () => {
  expect(release.version(join(import.meta.dir, ".."))).toBe("11.0.0-dev");
});

test("packing a clone that was never built says so, naming the missing build", () => {
  const dir = mkdtempSync(join(root, "unbuilt-"));
  mkdirSync(join(dir, "deploy"), { recursive: true });
  expect(() => release.collect(dir, "2026.11.16")).toThrow(
    /no .*target\/dev\/release\/svanbot: build the release first/,
  );
});

test("a version that is a path, not a version, is refused before it names a file", () => {
  // A git tag may contain `/`; the version becomes the bundle's file name, so it must be `N.N.N`.
  expect(() => release.safe("1.2.3/../../tmp/evil")).toThrow(/not <major>.<minor>.<patch>/);
  expect(() => release.safe("1.2.3-rc1")).toThrow(/not <major>.<minor>.<patch>/);
  expect(release.safe("2026.11.16")).toBe("2026.11.16");
});

test("a bundle writes the archive and its checksum beside it, named for the version and the cpu", () => {
  const out = join(root, "dist");
  const built = release.build({
    root: fakeClone(),
    version: "2026.11.16",
    cpu: "x86-64-v2",
    out,
  });
  expect(built.archive.endsWith("svanbot-2026.11.16-x86-64-v2.tar.gz")).toBe(true);
  expect(readFileSync(`${built.archive}.sha256`, "utf8")).toBe(
    `${built.sha256}  svanbot-2026.11.16-x86-64-v2.tar.gz\n`,
  );
  expect(release.sha256(readFileSync(built.archive))).toBe(built.sha256);
});
