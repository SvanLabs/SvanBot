// The gate: the checks a change passes, in tiers (V11-03). The commit tier is the fast one, run
// by the pre-commit hook. The full tier runs before a push and adds the workspace and tool tests.
//   bun tools/gate.ts commit   fmt, the 500-line limit, the secret scan, provenance, the layering
//                              check and clippy
//   bun tools/gate.ts full     the commit tier, plus the tests
import { join } from "node:path";

export type Step = { name: string; argv: string[]; env: Record<string, string> };

// Fast checks first, so a typo fails in a second rather than after a build.
const COMMIT = ["fmt", "file-size", "secrets", "provenance", "layering", "clippy"];
const TIERS: Record<string, string[]> = {
  commit: COMMIT,
  full: [...COMMIT, "cargo-test", "tools-test"],
};

const ROOT = join(import.meta.dir, "..");
const TARGET = { CARGO_TARGET_DIR: "target/dev" };

// Cargo runs at low priority, on four jobs, one build at a time. The fleet shares this box.
function cargo(args: string[]): string[] {
  const split = args.indexOf("--");
  const head = split < 0 ? args : args.slice(0, split);
  const tail = split < 0 ? [] : args.slice(split);
  return ["nice", "-n", "15", "ionice", "-c3", "cargo", ...head, "-j4", ...tail];
}

function stepFor(name: string): Step {
  switch (name) {
    case "fmt":
      return { name, argv: ["cargo", "fmt", "--all", "--check"], env: {} };
    case "file-size":
    case "secrets":
    case "provenance":
    case "layering":
      return { name, argv: [process.execPath, `tools/${name}.ts`], env: {} };
    case "clippy":
      return {
        name,
        argv: cargo(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
        env: TARGET,
      };
    case "cargo-test":
      return { name, argv: cargo(["test", "--workspace"]), env: TARGET };
    case "tools-test":
      return { name, argv: [process.execPath, "test", "tools"], env: {} };
    default:
      throw new Error(`no step named ${name}`);
  }
}

export function plan(tier: string): Step[] {
  if (!Object.hasOwn(TIERS, tier)) throw new Error(`unknown tier "${tier}": use commit or full`);
  return (TIERS[tier] ?? []).map(stepFor);
}

function run(step: Step): number {
  const result = Bun.spawnSync({
    cmd: step.argv,
    cwd: ROOT,
    env: { ...process.env, ...step.env },
    stdout: "inherit",
    stderr: "inherit",
  });
  return result.exitCode;
}

if (import.meta.main) {
  const tier = process.argv[2] ?? "";
  let steps: Step[];
  try {
    steps = plan(tier);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(2);
  }
  for (const step of steps) {
    console.log(`gate ${tier}: ${step.name}`);
    const code = run(step);
    if (code !== 0) {
      console.error(`gate ${tier}: ${step.name} failed (exit ${code})`);
      process.exit(code);
    }
  }
  console.log(`gate ${tier}: all ${steps.length} steps passed`);
}
