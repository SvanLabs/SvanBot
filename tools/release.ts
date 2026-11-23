// The release bundle: the one program the fleet runs, packed for a machine with no Rust toolchain.
// CI builds it on a `v*` tag, and the updater verifies and installs it before any slot is swapped.
//   bun tools/release.ts [--version V] [--cpu x86-64-vN] [--root DIR] [--out DIR]
// Writes <out>/svanbot-<version>-<cpu>.tar.gz and the tarball's `.sha256` beside it.
//
// The program is built by the workflow, not here: this runs after `cargo build --release` and only
// packages what is already in `target/dev/release`. `--cpu` names the instruction level the build
// was made for (`RUSTFLAGS=-C target-cpu=<level>` in the workflow); it never selects one. The level
// goes in the bundle's file name and in `UPGRADE/cpu`, which `install.sh` reads back and refuses
// when the machine's CPU lacks it: a bot that reaches an instruction its CPU does not have dies.
//
// The archive's shape is the contract with two readers: `release/` is the staged release the
// updater's `install` verifies (`crates/runtime/src/release.rs` — the program, its `sha256` and the
// `version` manifest), and `UPGRADE/` carries what a fresh machine needs and has no repo to take
// from. `install.sh` unpacks it and stages `release/`.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** The program's name, in the build tree and in `release/`. */
export const PROGRAM = "svanbot";

/** The updater's binary, which is the operator's tool and travels in `UPGRADE/`, not in a slot. */
export const UPDATER = "svanbot-release";

/** The directory the updater reads as a staged release. */
export const RELEASE = "release";

/** What a machine with no toolchain takes: the installer, and the tool that does the swapping. */
export const UPGRADE = "UPGRADE";

/**
 * The instruction level a release is built for: the box's. Its QEMU CPU reports every x86-64-v2
 * flag (SSE4.2, POPCNT, SSSE3, CX16) and none of v3's (no AVX, AVX2, BMI or FMA).
 */
export const CPU = "x86-64-v2";

/** The version manifest's name inside `release/`, which `Release::at` reads. */
export const MANIFEST = "version";

/** Files a bundle member is, keyed by its path in the archive. */
export type Files = Map<string, Buffer>;

/** The SHA-256 of `bytes`, lowercase hex, the form the release publishes. */
export function sha256(bytes: Buffer): string {
  const hash = new Bun.CryptoHasher("sha256");
  hash.update(bytes);
  return hash.digest("hex");
}

/** The version in the workspace manifest, which a `--version` overrides with the tag's. */
export function version(root: string): string {
  const manifest = readFileSync(join(root, "Cargo.toml"), "utf8");
  const section = manifest.split(/^\[/m).find((part) => part.startsWith("workspace.package]"));
  const line = section?.split("\n").find((entry) => entry.trim().startsWith("version"));
  const found = line?.match(/"([^"]+)"/)?.[1];
  if (!found) throw new Error(`no workspace version in ${join(root, "Cargo.toml")}`);
  return found;
}

/**
 * Everything that goes in the archive. The two programs must exist already; the release program's
 * checksum is computed here and written both as `release/sha256` and into the version manifest, so
 * the two readers of the release cannot disagree about which bytes were released.
 */
export function collect(root: string, version: string, cpu: string = CPU): Files {
  const program = built(root, PROGRAM);
  const digest = sha256(program);
  const files: Files = new Map();
  files.set(`${RELEASE}/${PROGRAM}`, program);
  files.set(`${RELEASE}/sha256`, Buffer.from(`${digest}  ${PROGRAM}\n`));
  files.set(`${RELEASE}/${MANIFEST}`, Buffer.from(`version\t${version}\nsha256\t${digest}\n`));
  for (const [member, path] of upgrade(root)) {
    if (!existsSync(path)) throw new Error(`no ${path}: build the release first`);
    files.set(`${UPGRADE}/${member}`, readFileSync(path));
  }
  // The level the program was built for, which `install.sh` checks against the machine's CPU.
  files.set(`${UPGRADE}/cpu`, Buffer.from(`${cpu}\n`));
  return files;
}

/** A program from the release build, or a message naming the build that was not run. */
function built(root: string, name: string): Buffer {
  const path = join(root, "target/dev/release", name);
  if (!existsSync(path)) throw new Error(`no ${path}: build the release first`);
  return readFileSync(path);
}

/**
 * What a fresh machine takes: the installer, the unit template, and the operator's tool. The
 * updater is not a slot program — nothing execs it from a build — so it lives here and is
 * installed once, beside the tree rather than in it.
 */
function upgrade(root: string): [string, string][] {
  return [
    ["install.sh", join(root, "deploy/install.sh")],
    ["svanbot@.service", join(root, "deploy/svanbot@.service")],
    [UPDATER, join(root, "target/dev/release", UPDATER)],
  ];
}

/**
 * `files` as a ustar archive: mode 0755 members, one 512-byte block per header, and two zero
 * blocks at the end. Written here rather than shelled out to `tar`, because the mode bits are the
 * contract — the program must arrive executable, or the updater refuses it and the unit cannot
 * exec it.
 */
export function tar(files: Files): Buffer {
  const parts: Buffer[] = [];
  for (const [path, bytes] of sorted(files)) {
    parts.push(header(path, bytes.length));
    parts.push(bytes);
    // Every member is padded to a 512-byte block; the padding is zeroed.
    parts.push(Buffer.alloc((512 - (bytes.length % 512)) % 512));
  }
  // Two zero blocks end the archive.
  parts.push(Buffer.alloc(1024));
  return Buffer.concat(parts);
}

/** The release archive: the tar, gzipped. */
export function pack(files: Files): Buffer {
  // gzip takes a plain ArrayBuffer-backed view; a `Buffer` is not one, so the tar is copied once.
  return Buffer.from(Bun.gzipSync(new Uint8Array(tar(files))));
}

/** The files in archive order, which `tar` requires to be sorted by path. */
function sorted(files: Files): [string, Buffer][] {
  return [...files.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/** One ustar header block: a regular file, mode 0755, owned by root, dated now. */
function header(path: string, size: number): Buffer {
  const block = Buffer.alloc(512);
  block.write(path, 0, "utf8");
  block.write("0000755\0", 100, "utf8"); // mode: the program and the installer both execute
  block.write("0000000\0", 108, "utf8"); // uid
  block.write("0000000\0", 116, "utf8"); // gid
  block.write(octal(size, 11), 124, "utf8"); // size
  block.write(octal(Math.floor(Date.now() / 1000), 11), 136, "utf8"); // mtime
  block.fill(0x20, 148, 156); // the checksum field is spaces while it is computed
  block.write("0", 156, "utf8"); // regular file
  block.write("ustar\0", 257, "utf8"); // magic and version (POSIX)
  block.write("00", 263, "utf8");
  block.write(checksum(block), 148, "utf8"); // six octal digits, NUL, space
  return block;
}

/** A tar numeric field: `width` octal digits, NUL-terminated. */
function octal(value: number, width: number): string {
  return `${value.toString(8).padStart(width, "0")}\0`;
}

/** The header's own checksum: the sum of its bytes with the checksum field read as spaces. */
function checksum(block: Buffer): string {
  let sum = 0;
  for (const byte of block) sum += byte;
  return `${sum.toString(8).padStart(6, "0")}\0 `;
}

/**
 * A version that is safe to name a file with: digits and dots, and the shape `N.N.N`. A git tag
 * may contain `/` and other characters a file name may not, and the version becomes the bundle's
 * name, so this is refused here as well as in the workflow — the check is what stops a tag like
 * `v1.2.3/../../x` from writing outside the output directory.
 */
export function safe(version: string): string {
  if (!/^[0-9]+\.[0-9]+\.[0-9]+$/.test(version)) {
    throw new Error(`version ${version} is not <major>.<minor>.<patch>`);
  }
  return version;
}

/** Packs a bundle and writes it, with its `.sha256`, under `out`. */
export function build(options: { root: string; version: string; cpu: string; out: string }): {
  archive: string;
  sha256: string;
} {
  const version = safe(options.version);
  const bytes = pack(collect(options.root, version, options.cpu));
  const name = `${PROGRAM}-${version}-${options.cpu}.tar.gz`;
  mkdirSync(options.out, { recursive: true });
  const archive = join(options.out, name);
  writeFileSync(archive, bytes);
  const digest = sha256(bytes);
  writeFileSync(`${archive}.sha256`, `${digest}  ${name}\n`);
  return { archive, sha256: digest };
}

function flag(args: string[], name: string): string | undefined {
  const at = args.indexOf(name);
  if (at < 0) return undefined;
  const value = args[at + 1];
  if (value === undefined) throw new Error(`${name} needs a value`);
  return value;
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const root = flag(args, "--root") ?? join(import.meta.dir, "..");
  const cpu = flag(args, "--cpu") ?? CPU;
  const forVersion = flag(args, "--version") ?? version(root);
  const out = flag(args, "--out") ?? join(root, "target/dist");
  const built = build({ root, version: forVersion, cpu, out });
  console.log(`${built.archive}  sha256 ${built.sha256}`);
}
