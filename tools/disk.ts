// The disk budget for build output. The box is shared with the live fleet and its store, so a
// clone's target/ stays under a budget, and the full tier reports it on every run.
//   bun tools/disk.ts [root]   prints target/'s size against the budget, and exits 1 if it is over
import { existsSync, lstatSync, readdirSync } from "node:fs";
import { join } from "node:path";

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

export function targetUse(root: string, budget = BUDGET): { over: boolean; line: string } {
  const target = join(root, "target");
  const bytes = existsSync(target) ? bytesUnder(target) : 0;
  if (bytes > budget) {
    return {
      over: true,
      line: `target/ holds ${size(bytes)}, over its ${size(budget)} budget: run \`cargo clean\` in this clone`,
    };
  }
  return { over: false, line: `target/ holds ${size(bytes)} of its ${size(budget)} budget` };
}

if (import.meta.main) {
  const { over, line } = targetUse(process.argv[2] ?? join(import.meta.dir, ".."));
  console.log(line);
  process.exit(over ? 1 : 0);
}
