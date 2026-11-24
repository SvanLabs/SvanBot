// The handoff note, tested at its one seam: the branch's state and what the session says in, the
// note the next agent reads out.
import { expect, test } from "bun:test";
import { note } from "./handoff";

const STATE = {
  ticket: 1176,
  branch: "v11-53-handoff",
  head: "abc1234",
  commits: ["abc1234 tools: the handoff note (V11-53)", "def5678 tools: its test (V11-53)"],
  dirty: [],
  pushed: true,
};

test("a note names the branch, its head and the commits ahead of main", () => {
  const text = note(STATE, { done: "The note and its test.", next: "Wire it into AGENTS.md." });
  expect(text).toContain("## Handoff: #1176");
  expect(text).toContain("`git fetch origin && git switch v11-53-handoff`");
  expect(text).toContain("at `abc1234`");
  expect(text).toContain(
    "- abc1234 tools: the handoff note (V11-53)\n- def5678 tools: its test (V11-53)",
  );
  expect(text).toContain("**Done:** The note and its test.");
  expect(text).toContain("**Next:** Wire it into AGENTS.md.");
  expect(text).toEndWith("Generated-by: claude-code/claude-opus-5-5\n");
});

test("work that is not on the remote is called out, since the next agent cannot see it", () => {
  const text = note(
    { ...STATE, pushed: false, dirty: ["M tools/handoff.ts", "?? tools/new.ts"] },
    { next: "Finish it." },
  );
  expect(text).toContain("**Not pushed:**");
  expect(text).toContain("**Uncommitted on the box:** M tools/handoff.ts, ?? tools/new.ts");
});

test("a branch with nothing ahead of main says so, and what the session did not say is asked for", () => {
  const text = note({ ...STATE, commits: [] }, {});
  expect(text).toContain("No commits ahead of `main` yet.");
  expect(text).toContain("**Done:** _not said_");
  expect(text).toContain("**Next:** _not said_");
});

test("the provenance line names the agent that wrote the note", () => {
  expect(note(STATE, {}, "codex/gpt-6")).toEndWith("Generated-by: codex/gpt-6\n");
});
