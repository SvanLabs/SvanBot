// The secret scan, tested at its one seam: a staged diff (and the .env beside it) in, findings out.
// Every fake secret is built from pieces at run time, so this file never trips the scan itself.
import { expect, test } from "bun:test";
import { scan } from "./secrets";

const ghToken = `ghp_${"a1".repeat(18)}`;

function diff(path: string, added: string[], firstLine = 1, removed: string[] = []): string {
  const hunk = `@@ -1,${removed.length} +${firstLine},${added.length} @@`;
  const lines = [`diff --git a/${path} b/${path}`, `--- a/${path}`, `+++ b/${path}`, hunk];
  const body = [...lines, ...removed.map((l) => `-${l}`), ...added.map((l) => `+${l}`)];
  return `${body.join("\n")}\n`;
}

test("a token on an added line is reported by its file and line, never by its value", () => {
  const added = ["let a = 1;", `let t = "${ghToken}";`];
  const findings = scan(diff("crates/runtime/src/client.rs", added, 10));
  expect(findings).toEqual(["crates/runtime/src/client.rs:11: a GitHub token"]);
  expect(findings.join("\n")).not.toContain(ghToken);
});

test("each kind of key the scan knows is caught", () => {
  const keys = [
    `github_pat_${"B".repeat(30)}`,
    `ghs_${"c".repeat(36)}`,
    `-----BEGIN ${"RSA PRIVATE"} KEY-----`,
    `sk-ant-${"d".repeat(30)}`,
    `AKIA${"E".repeat(16)}`,
  ];
  expect(scan(diff("notes.md", keys))).toEqual([
    "notes.md:1: a GitHub token",
    "notes.md:2: a GitHub token",
    "notes.md:3: a private key",
    "notes.md:4: an Anthropic API key",
    "notes.md:5: an AWS access key",
  ]);
});

test("a removed line is not reported", () => {
  expect(scan(diff("crates/core/src/lib.rs", [], 1, [`let t = "${ghToken}";`]))).toEqual([]);
});

test("staging an .env file is refused, but .env.example is not", () => {
  expect(scan(diff(".env", ["DEBUG=1"]))).toEqual([".env: an .env file is staged"]);
  expect(scan(diff("deploy/.env.local", ["DEBUG=1"]))).toEqual([
    "deploy/.env.local: an .env file is staged",
  ]);
  expect(scan(diff(".env.example", ["OPENPOKER_API_KEY="]))).toEqual([]);
});

test("a value from .env is caught by its variable's name, if the name says it is secret", () => {
  const key = "k".repeat(24);
  const env = [
    `export OPENPOKER_API_KEY="${key}"`,
    "LOG_LEVEL=debug-and-trace-all-day",
    "SHORT_TOKEN=abc",
  ].join("\n");
  const added = [`const key = "${key}";`, "level = debug-and-trace-all-day", "x = abc"];
  expect(scan(diff("crates/apps/svanbot/src/main.rs", added), env)).toEqual([
    "crates/apps/svanbot/src/main.rs:1: the value of OPENPOKER_API_KEY from .env",
  ]);
});
