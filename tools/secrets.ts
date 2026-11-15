// The secret scan of staged changes: no key, token or .env file reaches a commit. The repo is
// public, so a finding names the file, the line and the kind of secret, never the value.
//   bun tools/secrets.ts   scans `git diff --cached` against the known key shapes and the secret
//                          values in .env, and exits 1 on any finding
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

const KINDS: readonly (readonly [string, RegExp])[] = [
  ["a GitHub token", /\b(?:github_pat_[A-Za-z0-9_]{22,}|gh[pousr]_[A-Za-z0-9]{36,})/],
  ["a private key", /-----BEGIN [A-Z ]*PRIVATE KEY-----/],
  ["an Anthropic API key", /\bsk-ant-[A-Za-z0-9_-]{20,}/],
  ["an AWS access key", /\bAKIA[0-9A-Z]{16}\b/],
];

// A .env value counts when its name says it is secret and it is long enough not to match by chance.
const SECRET_NAME = /KEY|TOKEN|SECRET|PASS/;
const MIN_VALUE = 16;

function envSecrets(env: string): (readonly [string, string])[] {
  return env.split("\n").flatMap((raw) => {
    const match = /^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$/.exec(raw);
    if (!match) return [];
    const [, name = "", quoted = ""] = match;
    const value = quoted.replace(/^(["'])(.*)\1$/, "$2");
    return SECRET_NAME.test(name) && value.length >= MIN_VALUE ? [[name, value] as const] : [];
  });
}

const isEnvFile = (path: string) =>
  /(^|\/)\.env(\..+)?$/.test(path) && !path.endsWith(".env.example");

export function scan(diff: string, env = ""): string[] {
  const values = envSecrets(env);
  const findings: string[] = [];
  let file = "";
  let line = 0;
  let inHeader = false;
  for (const text of diff.split("\n")) {
    if (text.startsWith("diff --git ")) {
      inHeader = true;
      file = "";
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (hunk) {
      inHeader = false;
      line = Number(hunk[1]);
      continue;
    }
    if (inHeader) {
      if (text.startsWith("+++ b/")) {
        file = text.slice("+++ b/".length);
        if (isEnvFile(file)) findings.push(`${file}: an .env file is staged`);
      }
      continue;
    }
    if (text.startsWith("+")) {
      const added = text.slice(1);
      for (const [kind, shape] of KINDS) {
        if (shape.test(added)) findings.push(`${file}:${line}: ${kind}`);
      }
      for (const [name, value] of values) {
        if (added.includes(value)) findings.push(`${file}:${line}: the value of ${name} from .env`);
      }
      line += 1;
    } else if (text.startsWith(" ")) {
      line += 1;
    }
  }
  return findings;
}

if (import.meta.main) {
  const root = join(import.meta.dir, "..");
  const argv = ["git", "diff", "--cached", "-U0", "--no-color", "--no-ext-diff"];
  const staged = Bun.spawnSync(argv, { cwd: root });
  if (staged.exitCode !== 0) {
    console.error(`git diff --cached failed: ${staged.stderr.toString().trim()}`);
    process.exit(2);
  }
  const envPath = join(root, ".env");
  const env = existsSync(envPath) ? readFileSync(envPath, "utf8") : "";
  const findings = scan(staged.stdout.toString(), env);
  for (const finding of findings) console.log(finding);
  if (findings.length > 0) {
    console.log("secrets: commit refused. Unstage the secret, and rotate it if it was shared.");
  }
  process.exit(findings.length > 0 ? 1 : 0);
}
