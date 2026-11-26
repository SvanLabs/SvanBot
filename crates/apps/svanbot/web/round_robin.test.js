// The round-robin page (V11-97): the matrix, each bot's brain, check and guard, and the empty
// state before the first pass.

import { beforeAll, expect, test } from "bun:test";

import { h } from "./dom.js";
import { roundRobinPage } from "./round_robin.js";
import { standIn } from "./testdom.js";

const NOW = 1_800_000_000_000;

beforeAll(() => {
  globalThis.document = standIn;
});

function fleet() {
  return {
    round_robin: {
      empty: false,
      error: null,
      recorded_ms: NOW - 120_000,
      seed: 7,
      bouts: 1,
      iterations: null,
      budget: 60,
      played: 2,
      skipped: 58,
      columns: [
        {
          bot: "Alpha",
          policy: "solving/tight",
          champion: "308e7da4b29fd533",
          regression: "admitted",
          measured: true,
          brain: { kind: "solving", style: "tight" },
          check: {
            admitted: true,
            stale: false,
            checked_ms: NOW - 60_000,
            bouts: 60,
          },
          guard: { state: "watching", since_ms: NOW - 300_000 },
        },
        {
          bot: "Beta",
          policy: "solving/loose",
          champion: "1111222233334444",
          regression: "unresolved",
          measured: false,
          brain: null,
          check: null,
          guard: null,
        },
      ],
      matrix: [
        {
          row: "nit",
          table: "heads-up",
          archive: null,
          cells: [{ mean: 3_000, radius: 500, bouts: 40, verdict: "ahead" }, null],
        },
        {
          row: "archive 0000000000000001",
          table: "6-max",
          archive: "0000000000000001",
          cells: [null, { mean: 0, radius: null, bouts: 1, verdict: "unresolved" }],
        },
      ],
    },
  };
}

function page(data) {
  return h("div", {}, roundRobinPage(data, NOW));
}

test("the board shows the matrix and each bot's brain, check and guard", () => {
  const view = page(fleet());
  expect(view.textContent).toContain("Round-robin");
  expect(view.textContent).toContain("seed 7");
  expect(view.textContent).toContain("2 of 60 rows played, 58 left");

  const sections = view.all("section");
  expect(sections.map((section) => section.querySelector("h2").textContent)).toEqual([
    "Matrix",
    "Brains",
  ]);

  const matrix = view.all("table")[0];
  const heads = matrix.all("tr")[0];
  expect(heads.textContent).toContain("Alpha");
  expect(heads.textContent).toContain("Beta");
  const rows = matrix.all("tr").slice(1);
  expect(rows.length).toBe(2);
  expect(rows[0].textContent).toContain("nit");
  expect(rows[0].textContent).toContain("heads-up");
  expect(rows[0].textContent).toContain("ahead");
  expect(rows[0].textContent).toContain("+3000.0 mbb/100");
  // Alpha has no read against Beta's archive row, and Beta has none against Alpha's.
  expect(rows[0].all("td")[3].textContent).toBe("—");
  expect(rows[1].all("td")[2].textContent).toBe("—");
  expect(rows[1].textContent).toContain("archive 00000000");
  expect(rows[1].textContent).toContain("no reading");

  const brains = view.all("table")[1];
  expect(brains.textContent).toContain("Alpha");
  expect(brains.textContent).toContain("solving/tight");
  expect(brains.textContent).toContain("admitted · 1m ago");
  expect(brains.textContent).toContain("watching since 5m ago");
  expect(brains.textContent).toContain("Beta");
  expect(brains.textContent).toContain("reference policy");
  expect(brains.textContent).toContain("matrix for solving/loose");
  expect(brains.textContent).toContain("no check");
  expect(brains.textContent).toContain("not assigned");
});

test("an absent record is an empty board with the command that makes one", () => {
  const view = page({ round_robin: { empty: true, error: null, columns: [], matrix: [] } });
  expect(view.textContent).toContain("No round-robin pass is recorded yet.");
  expect(view.textContent).toContain("svanbot round-robin");
  expect(view.textContent).not.toContain("Matrix");
});

test("a record that cannot be read says why, not just that it is empty", () => {
  const view = page({
    round_robin: {
      empty: true,
      error: "round-robin.json: the record lacks bots",
      columns: [],
      matrix: [],
    },
  });
  expect(view.textContent).toContain("cannot be read");
  expect(view.textContent).toContain("round-robin.json: the record lacks bots");
});
