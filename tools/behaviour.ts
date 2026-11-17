// Whether a change can change how the bots play, so the Arena re-runs on it (V11-15). Play is
// decided in the pure layers: core, sim, strategy and learn. Runtime and apps wire them to the
// venue, the store and the clock, and a change there alone cannot move a scoreboard line.
//   bun tools/behaviour.ts <path>...
//   git diff --name-only | bun tools/behaviour.ts
//                                      prints the paths that can change play, one a line, and
//                                      nothing else. It always exits 0: "no pure path" is a
//                                      verdict, not an error, so the caller can tell a skip from
//                                      a failure. The paths come from stdin, one a line, so a big
//                                      diff is never split into batches, where one impure batch
//                                      could hide a pure.
const PURE = ["crates/core/", "crates/sim/", "crates/strategy/", "crates/learn/"];

/**
 * The paths among `paths` that can change how the bots play: a file in a pure layer, as a repo
 * path. Empty means the change cannot move a scoreboard line.
 */
export function behaviourPaths(paths: string[]): string[] {
  return paths
    .map((path) => path.replaceAll("\\", "/"))
    .filter((path) => PURE.some((layer) => path.startsWith(layer)));
}

if (import.meta.main) {
  // Paths as arguments, or one a line on stdin; an empty line is no path, not the root.
  const args = process.argv.slice(2);
  const paths =
    args.length > 0 ? args : (await Bun.stdin.text()).split("\n").filter((line) => line !== "");
  for (const path of behaviourPaths(paths)) console.log(path);
}
