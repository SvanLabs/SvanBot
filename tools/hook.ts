// The agent hooks: rules of AGENTS.md that a machine can hold an agent to (V11-12).
// `.claude/settings.json` wires them into Claude Code; each reads the hook's JSON on stdin.
//   bun tools/hook.ts pre-bash    before a shell command: exit 2 refuses it, with the reason
//   bun tools/hook.ts post-edit   after an edit: a changed Cargo.toml runs the layering check
//   bun tools/hook.ts stop        before the agent stops: a changed tree must pass the commit tier
// Exit 2 with a reason on stderr is how a hook says no: the agent reads the reason and goes on.
// A hook that cannot read its input lets the call through: it must never wedge a session.
import { join } from "node:path";

const ROOT = join(import.meta.dir, "..");

/** The cargo subcommands that compile. */
const BUILDS = /^(build|b|check|c|test|t|run|r|clippy|nextest|bench|doc|rustc|install)$/;

/** A command line cut at `&&`, `||`, `;` and `|`, each part as its words. */
function parts(command: string): string[][] {
  return command
    .split(/&&|\|\||[;|\n]/)
    .map((part) => part.trim().split(/\s+/).filter(Boolean))
    .filter((words) => words.length > 0);
}

/** The words of `words` after its leading `NAME=value` settings and wrappers like `nice -n 15`. */
function program(words: string[]): { env: Record<string, string>; argv: string[] } {
  const env: Record<string, string> = {};
  let at = 0;
  for (; at < words.length; at += 1) {
    const word = words[at] ?? "";
    const setting = /^([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(word);
    if (setting) {
      env[setting[1] ?? ""] = setting[2] ?? "";
    } else if (word === "nice" || word === "ionice" || word === "env" || word === "sudo") {
      // The wrapper's own flags and their values go with it.
      while (/^-/.test(words[at + 1] ?? "")) {
        at += /^-(n|c|p)$/.test(words[at + 1] ?? "") ? 2 : 1;
      }
    } else if (word === "mise" && words[at + 1] === "exec") {
      at = words.indexOf("--", at) < 0 ? words.length : words.indexOf("--", at);
    } else {
      break;
    }
  }
  return { env, argv: words.slice(at) };
}

/**
 * Why `command` may not run, or null when it may. `env` is the environment it would run in.
 */
export function refusal(command: string, env: Record<string, string | undefined>): string | null {
  for (const words of parts(command)) {
    const { env: set, argv } = program(words);
    const [tool, ...args] = argv;
    if (tool === "git" && args.includes("push")) {
      const forced = args.some((arg) => /^(-f|--force|--force-with-lease(=.*)?)$/.test(arg));
      const refs = args.filter((arg) => !arg.startsWith("-") && arg !== "push");
      if (refs.some((ref) => /^\+?([^:]*:)?(refs\/heads\/)?main$/.test(ref))) {
        return "main is pushed to only by a merged pull request: push the work to its own branch and open one into main.";
      }
      if (forced || refs.some((ref) => ref.startsWith("+"))) {
        return "A push never rewrites a branch here: no force. Add a commit, or open a new branch.";
      }
    }
    if (tool === "cargo") {
      const sub = args.find((arg) => !arg.startsWith("-") && !arg.startsWith("+"));
      if (sub && BUILDS.test(sub)) {
        if (args.includes("--release") || args.includes("-r")) {
          return "CI builds what the fleet runs, on a v* tag. No release build on this box: it shares its cores and memory with the live fleet.";
        }
        if ((set.CARGO_TARGET_DIR ?? env.CARGO_TARGET_DIR) !== "target/dev") {
          return "Build with CARGO_TARGET_DIR=target/dev, at low priority, one build at a time: `nice -n 15 ionice -c3 cargo … -j4`, or `mise run full`.";
        }
      }
    }
    const starts =
      tool === "systemctl" &&
      args.some((arg) => /^(start|stop|restart|enable|disable|reload-or-restart)$/.test(arg)) &&
      args.some((arg) => /svanbot/.test(arg));
    const swaps = tool === "svanbot-release" && args.some((arg) => /^(update|rollback)$/.test(arg));
    if (starts || swaps) {
      return "Starting, stopping or swapping a bot is the operator's move: a new connection takes over that bot's live session. Say what should run, and let the operator run it.";
    }
  }
  return null;
}

/** Whether an edit of `path` changed a crate manifest, so the layering check should run. */
export function touchesManifest(path: string): boolean {
  return /(^|\/)Cargo\.toml$/.test(path);
}

function run(argv: string[]): { code: number; output: string } {
  const result = Bun.spawnSync({ cmd: argv, cwd: ROOT, stdout: "pipe", stderr: "pipe" });
  return { code: result.exitCode, output: `${result.stdout}${result.stderr}` };
}

function refuse(reason: string): never {
  console.error(reason);
  process.exit(2);
}

if (import.meta.main) {
  let input: {
    tool_input?: { command?: string; file_path?: string };
    stop_hook_active?: boolean;
  } = {};
  try {
    input = JSON.parse(await Bun.stdin.text());
  } catch {
    process.exit(0);
  }
  switch (process.argv[2]) {
    case "pre-bash": {
      const reason = refusal(input.tool_input?.command ?? "", process.env);
      if (reason !== null) {
        refuse(reason);
      }
      break;
    }
    case "post-edit": {
      if (touchesManifest(input.tool_input?.file_path ?? "")) {
        const check = run([process.execPath, "tools/layering.ts"]);
        if (check.code !== 0) {
          refuse(`A Cargo.toml changed and the layering check now fails:\n${check.output}`);
        }
      }
      break;
    }
    case "stop": {
      // Once the gate has sent the agent back, it may stop the next time: no loop.
      if (input.stop_hook_active) {
        break;
      }
      const changed = run(["git", "status", "--porcelain"]);
      if (changed.code === 0 && changed.output.trim() !== "") {
        const gate = run([process.execPath, "tools/gate.ts", "commit"]);
        if (gate.code !== 0) {
          refuse(
            `The tree has changes that fail the gate's commit tier. Fix them, or say plainly that they are left failing:\n${gate.output.slice(-3000)}`,
          );
        }
      }
      break;
    }
    default:
      console.error("hook: use pre-bash, post-edit or stop");
  }
}
