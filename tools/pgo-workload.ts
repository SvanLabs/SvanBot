// V11-123: the fixed-seed workload that trains PGO. It runs the same commands the paired
// benchmark measures, so the profile sees the sims the fleet's checks spend their time in.
//   bun tools/pgo-workload.ts <svanbot>
// The binary is the instrumented release build; its raw profiles land in the directory its
// `-Cprofile-generate` names, and the workflow merges them with `llvm-profdata`.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** The fixed-seed workload, in the order it runs. Both leagues, the paired sim and the Arena. */
export const workload = [
  ["paired", "2000", "7"],
  ["arena", "2", "7", "20"],
  ["arena", "6-max", "2", "7", "20"],
] as const;

if (import.meta.main) {
  const [bin] = Bun.argv.slice(2);
  if (!bin) {
    console.error("usage: bun tools/pgo-workload.ts <svanbot>");
    process.exit(2);
  }
  const fleet = mkdtempSync(join(tmpdir(), "svanbot-pgo-"));
  try {
    for (const args of workload) {
      const run = Bun.spawnSync({
        cmd: [bin, ...args],
        env: { ...process.env, SVANBOT_FLEET: fleet } as Record<string, string>,
        stdout: "ignore",
        stderr: "inherit",
      });
      if (run.exitCode !== 0) {
        console.error(`${args.join(" ")} exited ${run.exitCode}`);
        process.exit(1);
      }
    }
  } finally {
    rmSync(fleet, { recursive: true, force: true });
  }
}
