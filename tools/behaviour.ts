// Whether a change can change how the bots play, so the Arena re-runs on it (V11-15). Play is
// decided in the pure layers: core, sim, strategy and learn. Runtime and apps wire them to the
// venue, the store and the clock, and a change there alone cannot move a scoreboard line.
//   bun tools/behaviour.ts <path>...   exits 0 when the change can change behaviour, 1 when it
//                                      cannot, so a workflow can gate the Arena run on it.
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
  const changed = behaviourPaths(process.argv.slice(2));
  for (const path of changed) console.log(path);
  process.exit(changed.length > 0 ? 0 : 1);
}
