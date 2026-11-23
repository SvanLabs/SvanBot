// The provenance rule, tested at its one seam: commits in, the ones that do not name the system
// that produced them out. Who authored a commit does not matter: v11's own commits are authored by
// svanlabs[bot], so an exemption for bot authors would exempt everything.
import { expect, test } from "bun:test";
import { rangeFor, unattributed } from "./provenance";

test("a pull request is checked since the branch it merges into, anything else since main", () => {
  expect(rangeFor("v11")).toBe("origin/v11..HEAD");
  expect(rangeFor(undefined)).toBe("origin/main..HEAD");
  // A push run sets the variable, empty.
  expect(rangeFor("")).toBe("origin/main..HEAD");
});

const commit = (sha: string, message: string, parents = 1) => ({ sha, message, parents });

test("a commit that names its tool, with or without the model, passes", () => {
  expect(
    unattributed([
      commit("a".repeat(40), "core: cards\n\nGenerated-by: claude-code/claude-opus-5-5\n"),
      commit("b".repeat(40), "docs: a typo\n\nGenerated-by: codex\nCo-Authored-By: someone\n"),
    ]),
  ).toEqual([]);
});

test("a commit without the line is reported by its short id and subject", () => {
  expect(unattributed([commit("c".repeat(40), "sim: deal\n\nThe body.\n")])).toEqual([
    "ccccccc (sim: deal) has no Generated-by: <tool>/<model> line",
  ]);
});

test("a line whose value is not a tool name does not count", () => {
  const messages = [
    "x\n\nGenerated-by: claude code\n",
    "y\n\nGenerated-by:\n",
    "z\n\n Generated-by: a/b\n",
  ];
  expect(unattributed(messages.map((m, i) => commit(`${i}`.repeat(40), m)))).toEqual([
    "0000000 (x) has no Generated-by: <tool>/<model> line",
    "1111111 (y) has no Generated-by: <tool>/<model> line",
    "2222222 (z) has no Generated-by: <tool>/<model> line",
  ]);
});

test("a merge commit is skipped: it carries no change of its own", () => {
  expect(unattributed([commit("d".repeat(40), "Merge pull request #1 from x\n", 2)])).toEqual([]);
});
