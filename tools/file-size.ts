// The 500-line limit on Rust source files: a file that long has more than one job, so split it by
// responsibility before it grows past the limit.
//   bun tools/file-size.ts [root]   checks each .rs file under root/crates; exits 1 if one is over
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const LIMIT = 500;

function rustFiles(dir: string): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "target" ? [] : rustFiles(path);
    return entry.isFile() && entry.name.endsWith(".rs") ? [path] : [];
  });
}

function lineCount(text: string): number {
  const newlines = text.split("\n").length - 1;
  return text.length > 0 && !text.endsWith("\n") ? newlines + 1 : newlines;
}

export function oversized(root: string): string[] {
  return rustFiles(join(root, "crates"))
    .map((path) => ({
      path: relative(root, path).split("\\").join("/"),
      lines: lineCount(readFileSync(path, "utf8")),
    }))
    .filter((file) => file.lines > LIMIT)
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0))
    .map((file) => `${file.path} has ${file.lines} lines; the limit is ${LIMIT}`);
}

if (import.meta.main) {
  const problems = oversized(process.argv[2] ?? join(import.meta.dir, ".."));
  for (const problem of problems) console.log(problem);
  process.exit(problems.length > 0 ? 1 : 0);
}
