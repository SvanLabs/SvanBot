// The disk budget for build output. The box is shared with the live fleet and its store, so a
// clone's target/ directories stay under one budget, and the full tier reports it on every run.
//   bun tools/disk.ts   reports the combined size of every target/ directory in this clone, and
//                       exits 1 if it is over. It measures the clone it sits in, and takes no path.
import { existsSync, lstatSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// A tenth of the box's 93 GiB disk.
export const BUDGET = 9 * 2 ** 30;

function size(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB"] as const;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} ${units[0]}` : `${value.toFixed(1)} ${units[unit]}`;
}

// Apparent sizes, with each inode counted once: cargo hard-links its outputs out of deps/.
function bytesUnder(dir: string, seen = new Set<string>()): number {
  let total = 0;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      total += bytesUnder(path, seen);
    } else if (entry.isFile()) {
      const stat = lstatSync(path);
      const inode = `${stat.dev}:${stat.ino}`;
      if (!seen.has(inode)) {
        seen.add(inode);
        total += stat.size;
      }
    }
  }
  return total;
}

// Neither holds a cargo build, and both are large to walk.
const UNSEARCHED = new Set([".git", "node_modules"]);

// Every target/ under the root, worktrees' and nested crates' included: each is disk the fleet
// shares. Symlinks are not followed, so a link cannot count a tree twice or leave the clone.
function targetDirectoriesUnder(root: string): string[] {
  const targets: string[] = [];
  function collect(dir: string): void {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (!entry.isDirectory() || UNSEARCHED.has(entry.name)) continue;
      const path = join(dir, entry.name);
      if (entry.name === "target") {
        targets.push(path);
      } else {
        collect(path);
      }
    }
  }
  collect(root);
  return targets;
}

export function targetUse(root: string, budget = BUDGET): { over: boolean; line: string } {
  const targets =
    existsSync(root) && statSync(root).isDirectory() ? targetDirectoriesUnder(root) : [];
  const seen = new Set<string>();
  const sizes = targets
    .map((target) => ({ target: relative(root, target), bytes: bytesUnder(target, seen) }))
    .sort((a, b) => b.bytes - a.bytes || a.target.localeCompare(b.target));
  const bytes = sizes.reduce((total, { bytes }) => total + bytes, 0);
  if (bytes > budget) {
    const trees = sizes.map(({ target, bytes }) => `\n  ${size(bytes)} ${target}`).join("");
    return {
      over: true,
      line: `target/ directories hold ${size(bytes)}, over this clone's ${size(budget)} budget: remove build output from every target/ directory in this clone${trees}`,
    };
  }
  return {
    over: false,
    line: `target/ directories hold ${size(bytes)} of this clone's ${size(budget)} budget`,
  };
}

if (import.meta.main) {
  const { over, line } = targetUse(join(import.meta.dir, ".."));
  console.log(line);
  process.exit(over ? 1 : 0);
}
