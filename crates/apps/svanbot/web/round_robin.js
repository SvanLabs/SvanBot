// The round-robin board (V11-97): the matrix of each bot's gauntlet, and each bot's brain, check
// and guard. The server reads `round-robin.json` and the fleet's policy files; this only lays it
// out. A record that is absent is an empty board with the command that makes one.

import { h, panel, table } from "./dom.js";
import { duration, mbbText } from "./fleet.js";

const VERDICT = {
  ahead: "ahead",
  behind: "behind",
  even: "even",
  unresolved: "no reading",
};

/** How long ago `ms`, or "never" when there is no time. */
function ago(ms, nowMs) {
  return ms === null || ms === undefined ? "never" : `${duration(nowMs - ms)} ago`;
}

/** A matrix cell's read, or a dash when this bot's plan had no such row. */
function cell(value, bot) {
  if (!value) {
    return h("td", { class: "number muted" }, "—");
  }
  const label = VERDICT[value.verdict] ?? value.verdict;
  const tone = value.verdict === "ahead" ? "up" : value.verdict === "behind" ? "down" : "muted";
  const radius = Number.isFinite(value.radius) ? value.radius : null;
  const detail =
    `${bot}: ${mbbText(value.mean)}` +
    (radius === null ? "" : ` ±${radius.toFixed(0)}`) +
    ` over ${value.bouts} bout${value.bouts === 1 ? "" : "s"}, ${label}`;
  return h(
    "td",
    { class: "number", title: detail },
    h("span", { class: tone }, label),
    h("span", { class: "mono" }, ` ${mbbText(value.mean)}`),
    radius === null ? null : h("span", { class: "muted" }, ` ±${radius.toFixed(0)}`),
  );
}

/** The matrix: one row a gauntlet row, one column a bot, each cell that bot's read. */
function matrixTable(board) {
  const columns = board.columns;
  return table(
    "Each gauntlet row and every bot's read against it",
    [["Row", null], ["Table", null], ...columns.map((column) => [column.bot, "number"])],
    board.matrix.map((row) =>
      h(
        "tr",
        {},
        h(
          "td",
          { class: row.archive === null ? "" : "mono" },
          row.archive === null ? row.row : `archive ${row.archive.slice(0, 8)}`,
        ),
        h("td", {}, row.table),
        row.cells.map((value, at) => cell(value, columns[at]?.bot ?? "")),
      ),
    ),
  );
}

/** A check's state, and how long ago it was recorded, or that there is none. */
function checkText(check, nowMs) {
  if (!check) {
    return "no check";
  }
  const state = check.admitted ? "admitted" : "not admitted";
  const stale = check.stale === true ? ", stale" : "";
  return `${state}${stale} · ${ago(check.checked_ms, nowMs)}`;
}

/** The guard's window for an assigned bot, or the reference policy's absence. */
function guardText(guard, nowMs) {
  return guard ? `${guard.state} since ${ago(guard.since_ms, nowMs)}` : "not assigned";
}

/** Each bot's brain, its latest check and the live guard's window. */
function brainsTable(board, nowMs) {
  return table(
    "Each bot's brain, check and guard",
    [
      ["Bot", null],
      ["Brain", null],
      ["Check", null],
      ["Guard", null],
      ["Regression", null],
    ],
    board.columns.map((column) =>
      h(
        "tr",
        {},
        h("td", {}, h("a", { href: `/bot/${encodeURIComponent(column.bot)}` }, column.bot)),
        h(
          "td",
          {},
          column.brain ? `${column.brain.kind}/${column.brain.style}` : "reference policy",
          column.measured === false
            ? h("span", { class: "muted" }, ` · matrix for ${column.policy}`)
            : null,
        ),
        h("td", {}, checkText(column.check, nowMs)),
        h("td", {}, guardText(column.guard, nowMs)),
        h("td", {}, column.regression),
      ),
    ),
  );
}

/** The round-robin page: its record, or the empty state that says how to make one. */
export function roundRobinPage(fleet, nowMs) {
  const board = fleet.round_robin;
  const empty =
    !board || board.empty === true || !Array.isArray(board.columns) || !Array.isArray(board.matrix);
  if (empty) {
    return [
      h("h1", {}, "Round-robin"),
      h(
        "p",
        {},
        board?.error
          ? `The round-robin record cannot be read: ${board.error}`
          : "No round-robin pass is recorded yet. Play one with ",
        board?.error ? null : h("code", {}, "svanbot round-robin"),
        board?.error ? null : ".",
      ),
    ];
  }
  const iterations =
    board.iterations === null || board.iterations === undefined
      ? "compute-budget iterations"
      : `${board.iterations} iterations`;
  return [
    h("h1", {}, "Round-robin"),
    h(
      "p",
      { class: "muted" },
      `Recorded ${ago(board.recorded_ms, nowMs)} · seed ${board.seed} · ` +
        `${board.bouts} bout${board.bouts === 1 ? "" : "s"} a row · ${iterations} · ` +
        `${board.played} of ${board.budget} rows played, ${board.skipped} left`,
    ),
    h(
      "div",
      { class: "grid" },
      panel(
        "Matrix",
        h(
          "p",
          { class: "muted" },
          "Each row is a gauntlet opponent and each column a bot's read against it, in " +
            "milli-big-blinds a hundred deals from that bot's seat. A verdict is the whole " +
            "95% bound off zero; no reading means fewer than two bouts.",
        ),
        h(
          "div",
          { class: "scroll", tabindex: 0 },
          board.matrix.length > 0
            ? matrixTable(board)
            : h("p", { class: "muted" }, "This pass recorded no rows."),
        ),
      ),
      panel(
        "Brains",
        h(
          "p",
          { class: "muted" },
          "The brain each bot plays now, its latest margin check against the champion in " +
            "play, and the live guard's window.",
        ),
        brainsTable(board, nowMs),
      ),
    ),
  ];
}
