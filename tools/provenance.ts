// Provenance: every commit names the system that produced it, in a `Generated-by: <tool>/<model>`
// line (the model may be left out when it is not known). This checks the declaration, not who
// wrote the code: the line is self-reported, and only review makes it true.
//   bun tools/provenance.ts [range]          checks each commit in range (default origin/main..HEAD)
//   bun tools/provenance.ts --message FILE   checks one commit message: the commit-msg hook
import { readFileSync } from "node:fs";
import { join } from "node:path";

export type Commit = { sha: string; message: string; parents: number };

const TRAILER = /^Generated-by:[ \t]*[A-Za-z0-9._+-]+(?:\/[A-Za-z0-9._+-]+)?[ \t]*$/m;
const MISSING = "has no Generated-by: <tool>/<model> line";

export const isAttributed = (message: string) => TRAILER.test(message);

export function unattributed(commits: readonly Commit[]): string[] {
  return commits
    .filter((c) => c.parents < 2 && !isAttributed(c.message))
    .map((c) => `${c.sha.slice(0, 7)} (${c.message.split("\n")[0]}) ${MISSING}`);
}

// Field and record separators: control characters no commit message contains.
const FIELD = "\x1f";
const RECORD = "\x1e";

/** The commits to check: on a pull request, those since the branch it merges into, else since main. */
export function rangeFor(baseRef: string | undefined): string {
  return `origin/${baseRef || "main"}..HEAD`;
}

function commitsIn(range: string, root: string): Commit[] {
  const format = `--format=%H${FIELD}%P${FIELD}%B${RECORD}`;
  const log = Bun.spawnSync(["git", "log", format, range], { cwd: root });
  if (log.exitCode !== 0) throw new Error(`git log ${range}: ${log.stderr.toString().trim()}`);
  return log.stdout
    .toString()
    .split(RECORD)
    .map((record) => record.replace(/^\n/, ""))
    .filter((record) => record.length > 0)
    .map((record) => {
      const [sha = "", parents = "", message = ""] = record.split(FIELD);
      return { sha, message, parents: parents.split(" ").filter(Boolean).length };
    });
}

if (import.meta.main) {
  const [first, second] = process.argv.slice(2);
  if (first === "--message") {
    if (!isAttributed(readFileSync(second ?? "", "utf8"))) {
      console.log(`the commit message ${MISSING}`);
      process.exit(1);
    }
    process.exit(0);
  }
  const root = join(import.meta.dir, "..");
  const problems = unattributed(commitsIn(first ?? rangeFor(process.env.GITHUB_BASE_REF), root));
  for (const problem of problems) console.log(problem);
  process.exit(problems.length > 0 ? 1 : 0);
}
