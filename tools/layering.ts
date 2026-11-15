// The layering rule of docs/adr/0003-crate-graph.md, checked from `cargo metadata`.
//
//   bun tools/layering.ts [root]   checks the workspace at root (default: this one), lists each
//                                  violation and exits 1 if there is any
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";

/** The layers, lowest first. A crate's layer is the directory under `crates/` that holds it. */
const LAYERS = ["core", "sim", "strategy", "learn", "runtime", "apps"] as const;
type Layer = (typeof LAYERS)[number];
/** The layers that never touch the network, the store or the clock. */
const PURE: readonly Layer[] = ["core", "sim", "strategy", "learn"];
/** The lints `clippy.toml` bans I/O through, and the groups and levels that contain them. */
const IO_BANS = [
  "clippy::disallowed_methods",
  "clippy::disallowed_types",
  "clippy::disallowed_macros",
  "clippy::style",
  "clippy::all",
  "warnings",
];

export type Violation = {
  crate: string;
  rule: "upward" | "unplaced" | "third-party" | "io-opt-out";
  detail: string;
};

type Dependency = { name: string; kind: "dev" | "build" | null; path?: string };
type Package = { name: string; manifest_path: string; dependencies: Dependency[] };
type Metadata = {
  packages: Package[];
  workspace_root: string;
  metadata: { layering?: { "pure-third-party"?: Record<string, string> } } | null;
};

export function checkWorkspace(root: string): Violation[] {
  const metadata = cargoMetadata(root);
  const layerOf = (manifestDir: string): Layer | undefined => {
    const [top, name] = relative(metadata.workspace_root, manifestDir).split(sep);
    return top === "crates" ? LAYERS.find((layer) => layer === name) : undefined;
  };
  const allowed = metadata.metadata?.layering?.["pure-third-party"] ?? {};
  const byDir = new Map(metadata.packages.map((p) => [dirname(p.manifest_path), p]));
  const violations: Violation[] = [];
  // Sorted by code unit, not locale, so the report reads the same on every machine.
  const packages = [...metadata.packages].sort((a, b) => (a.name < b.name ? -1 : 1));
  for (const pkg of packages) {
    const report = (rule: Violation["rule"], detail: string) =>
      violations.push({ crate: pkg.name, rule, detail });
    const dir = dirname(pkg.manifest_path);
    const layer = layerOf(dir);
    if (layer === undefined) {
      report("unplaced", `${relative(metadata.workspace_root, dir)} is not under crates/<layer>/`);
      continue;
    }
    if (PURE.includes(layer)) {
      for (const [file, lint] of ioOptOuts(dir)) report("io-opt-out", `${file} turns off ${lint}`);
    }
    for (const dep of pkg.dependencies) {
      // Anything that is not a workspace member is third-party, a path outside the workspace too.
      const target = dep.path === undefined ? undefined : byDir.get(dep.path);
      if (target === undefined) {
        if (PURE.includes(layer) && dep.kind !== "dev" && !Object.hasOwn(allowed, dep.name)) {
          const kind = dep.kind === "build" ? " (build)" : "";
          report("third-party", `${dep.name}${kind} in a pure layer (${layer})`);
        }
        continue;
      }
      const targetLayer = layerOf(dirname(target.manifest_path));
      if (targetLayer === undefined) continue; // reported as unplaced on its own
      if (LAYERS.indexOf(targetLayer) > LAYERS.indexOf(layer)) {
        report("upward", `${target.name} (${targetLayer}) is above ${layer}`);
      } else if (layer === "apps" && targetLayer === "apps") {
        report("upward", `${target.name} (apps) is another app`);
      }
    }
  }
  return violations;
}

/** Each `allow` or `expect` of an I/O ban in the crate's library code, as [file, lint]. */
function ioOptOuts(crateDir: string): [string, string][] {
  const src = join(crateDir, "src");
  if (!existsSync(src)) return [];
  const files = readdirSync(src, { recursive: true, encoding: "utf8" })
    .filter((file) => file.endsWith(".rs"))
    .sort();
  const found: [string, string][] = [];
  for (const file of files) {
    const text = readFileSync(join(src, file), "utf8");
    for (const attribute of text.matchAll(/#!?\[[^\]]*?\b(?:allow|expect)\s*\(([^)]*)\)/g)) {
      for (const lint of (attribute[1] ?? "").split(",").map((item) => item.trim())) {
        if (IO_BANS.includes(lint)) found.push([join("src", file), lint]);
      }
    }
  }
  return found;
}

function cargoMetadata(root: string): Metadata {
  const run = Bun.spawnSync(
    ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
    { cwd: root, stderr: "pipe" },
  );
  if (run.exitCode !== 0) throw new Error(`cargo metadata failed in ${root}:\n${run.stderr}`);
  return JSON.parse(run.stdout.toString()) as Metadata;
}

if (import.meta.main) {
  const violations = checkWorkspace(process.argv[2] ?? join(import.meta.dir, ".."));
  for (const v of violations) console.log(`${v.crate}: ${v.rule}: ${v.detail}`);
  process.exit(violations.length === 0 ? 0 : 1);
}
