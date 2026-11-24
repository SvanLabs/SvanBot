// The handoff note (V11-53): what a session leaves on its ticket when it stops mid-work, so that
// another agent (Claude Code, Codex, opencode or openclaude) can take the ticket over from the
// pushed branch. AGENTS.md has the protocol.
//
//   bun tools/handoff.ts <ticket> [--done "…"] [--next "…"] [--traps "…"] [--by <tool/model>]
//
// It prints the note; the session posts it (`gh issue comment <ticket> --body-file -`) and adds
// the label `agent-handoff`. It reads git and writes nothing.

/** The branch's state, read from git. */
export type State = {
  ticket: number;
  branch: string;
  head: string;
  /** The commits ahead of `origin/main`, newest first, as `git log --oneline` prints them. */
  commits: string[];
  /** Uncommitted paths, as `git status --porcelain` prints them. */
  dirty: string[];
  /** Whether `origin` holds the branch at `head`. */
  pushed: boolean;
};

/** What the session says, in its own words. */
export type Said = {
  done?: string | undefined;
  next?: string | undefined;
  traps?: string | undefined;
};

export const DEFAULT_BY = "claude-code/claude-opus-5-5";

export function note(state: State, said: Said, by: string = DEFAULT_BY): string {
  const lines = [
    `## Handoff: #${state.ticket}`,
    "",
    `Branch \`${state.branch}\` at \`${state.head}\`. To take it over: \`git fetch origin && git switch ${state.branch}\`, remove the label \`agent-handoff\`, and work on from here.`,
    "",
  ];
  if (!state.pushed) {
    lines.push(
      `**Not pushed:** \`origin\` does not hold \`${state.head}\`. Push it first, or the next agent starts without it.`,
      "",
    );
  }
  if (state.dirty.length > 0) {
    lines.push(`**Uncommitted on the box:** ${state.dirty.join(", ")}`, "");
  }
  lines.push(
    state.commits.length > 0
      ? `Commits ahead of \`main\`:\n${state.commits.map((commit) => `- ${commit}`).join("\n")}`
      : "No commits ahead of `main` yet.",
    "",
    `**Done:** ${said.done ?? "_not said_"}`,
    "",
    `**Next:** ${said.next ?? "_not said_"}`,
    "",
  );
  if (said.traps) lines.push(`**Traps:** ${said.traps}`, "");
  lines.push(`Generated-by: ${by}`, "");
  return lines.join("\n");
}

function git(...args: string[]): string {
  const result = Bun.spawnSync(["git", ...args], { stdout: "pipe", stderr: "pipe" });
  return result.exitCode === 0 ? result.stdout.toString().trim() : "";
}

function flag(argv: string[], name: string): string | undefined {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
}

if (import.meta.main) {
  const argv = process.argv.slice(2);
  const ticket = Number(argv[0]?.replace(/^#/, ""));
  if (!Number.isInteger(ticket) || ticket <= 0) {
    console.error('usage: bun tools/handoff.ts <ticket> [--done "…"] [--next "…"] [--traps "…"]');
    process.exit(2);
  }
  const branch = git("rev-parse", "--abbrev-ref", "HEAD");
  const head = git("rev-parse", "--short", "HEAD");
  const remote = git("ls-remote", "origin", `refs/heads/${branch}`).split(/\s/)[0] ?? "";
  const state: State = {
    ticket,
    branch,
    head,
    commits: git("log", "--oneline", "origin/main..HEAD").split("\n").filter(Boolean),
    dirty: git("status", "--porcelain").split("\n").filter(Boolean),
    pushed: remote !== "" && remote.startsWith(git("rev-parse", "HEAD").slice(0, remote.length)),
  };
  const said: Said = {
    done: flag(argv, "--done"),
    next: flag(argv, "--next"),
    traps: flag(argv, "--traps"),
  };
  process.stdout.write(note(state, said, flag(argv, "--by") ?? DEFAULT_BY));
}
