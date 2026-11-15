// The gate: the checks a change passes, in tiers (V11-03). The commit tier is the fast one, run
// by the pre-commit hook. The full tier runs before a push and adds the tests and the type check.
// Run it under mise (`mise run commit`, `mise run full`), so every tool is the pinned one.
//   bun tools/gate.ts commit   fmt, biome, the 500-line limit, the secret scan, provenance, the
//                              layering check and clippy
//   bun tools/gate.ts full     the commit tier, then nextest, the doc tests, tsc and the tool tests,
//                              and last the disk budget of this clone's target/
//   bun tools/gate.ts test     the full tier's tests alone: CI runs it beside the commit tier
//   bun tools/gate.ts ci       cargo-deny, run in CI on every pull request and once a week
import { join } from "node:path";

export type Step = { name: string; argv: string[]; env: Record<string, string> };

// Fast checks first, so a typo fails in a second rather than after a build.
const COMMIT = ["fmt", "biome", "file-size", "secrets", "provenance", "layering", "clippy"];
const TEST = ["nextest", "doc-test", "tsc", "tools-test"];
const TIERS: Record<string, string[]> = {
  commit: COMMIT,
  // The disk budget runs last, after the tests have built what they build. CI's disk is
  // thrown away after every run, so the test tier leaves it out.
  full: [...COMMIT, ...TEST, "disk"],
  test: TEST,
  ci: ["deny"],
};

const ROOT = join(import.meta.dir, "..");
const TARGET = { CARGO_TARGET_DIR: "target/dev" };

// Cargo runs at low priority, on four jobs, one build at a time. The fleet shares this box.
function cargo(args: string[], jobs = ["-j4"]): string[] {
  const split = args.indexOf("--");
  const head = split < 0 ? args : args.slice(0, split);
  const tail = split < 0 ? [] : args.slice(split);
  return ["nice", "-n", "15", "ionice", "-c3", "cargo", ...head, ...jobs, ...tail];
}

function stepFor(name: string): Step {
  switch (name) {
    case "fmt":
      return { name, argv: ["cargo", "fmt", "--all", "--check"], env: {} };
    case "biome":
      return { name, argv: ["biome", "check", "tools"], env: {} };
    case "file-size":
    case "secrets":
    case "provenance":
    case "layering":
    case "disk":
      return { name, argv: [process.execPath, `tools/${name}.ts`], env: {} };
    case "clippy":
      return {
        name,
        argv: cargo(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
        env: TARGET,
      };
    case "nextest":
      // nextest's own -j is its test threads, so the build jobs take their long flag.
      return {
        name,
        argv: cargo(
          ["nextest", "run", "--workspace", "--no-tests=fail"],
          ["--build-jobs", "4", "--test-threads", "4"],
        ),
        env: TARGET,
      };
    case "doc-test":
      return { name, argv: cargo(["test", "--workspace", "--doc"]), env: TARGET };
    case "tsc":
      return { name, argv: ["tools/node_modules/.bin/tsc", "-p", "tools"], env: {} };
    case "deny":
      return { name, argv: ["cargo", "deny", "--locked", "check"], env: {} };
    case "tools-test":
      return { name, argv: [process.execPath, "test", "tools"], env: {} };
    default:
      throw new Error(`no step named ${name}`);
  }
}

export function plan(tier: string): Step[] {
  if (!Object.hasOwn(TIERS, tier)) {
    const known = Object.keys(TIERS);
    throw new Error(
      `unknown tier "${tier}": use ${known.slice(0, -1).join(", ")} or ${known.at(-1)}`,
    );
  }
  return (TIERS[tier] ?? []).map(stepFor);
}

function run(step: Step): number {
  try {
    const result = Bun.spawnSync({
      cmd: step.argv,
      cwd: ROOT,
      env: { ...process.env, ...step.env },
      stdout: "inherit",
      stderr: "inherit",
    });
    return result.exitCode;
  } catch {
    // The tool is not installed: the spawn itself failed.
    console.error(`gate: cannot run ${step.argv[0]}. Run \`mise run setup\` once in this clone.`);
    return 127;
  }
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
  console.log(
    `gate ${tier}: ${steps.length === 1 ? "the step" : `all ${steps.length} steps`} passed`,
  );
}
