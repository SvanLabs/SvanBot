// The dossier (V11-26, `GLOSSARY.md`): one bot's view of each opponent it has played, from the
// aggregates its status carries. The raw counts never leave the bot's store; this page has only
// the class, the evidence and the share of the exploit budget each read has earned.

import { h, panel, table } from "./dom.js";

/** Actions at which a read earns half its class's share: the half-trust point. */
const HALF = 30;

/** How sure a read is from the actions in it: none, thin below the half-trust point, or read. */
export function band(actions) {
  if (actions <= 0) {
    return "no read";
  }
  return actions < HALF ? "thin" : "read";
}

/** The class as the page says it: a switch names the hand the present strategy began at. */
function classText(row) {
  return row.class === "switching" && Number.isFinite(row.change)
    ? `switching since hand ${row.change}`
    : row.class;
}

/** One bot's dossier: every opponent it reads, alphabetical by venue name, never ranked. */
export function dossierPage(status, name) {
  const title = h("h1", {}, `${name}: dossier`);
  if (!status) {
    return [title, h("p", {}, "This bot's dossier is empty: this bot has written no status.")];
  }
  const opponents = Array.isArray(status.opponents) ? [...status.opponents] : [];
  const copy = h(
    "p",
    { class: "muted" },
    "An opponent is known by their venue name, so a rename is a new opponent. The earned share ",
    "is how much of the bot's exploit budget the read may spend (the class's cap times n / (n + ",
    `${HALF})), not a measured harm.`,
  );
  if (opponents.length === 0) {
    return [title, copy, h("p", {}, "This bot has no reads yet: it has played no named opponent.")];
  }
  opponents.sort((a, b) => a.name.localeCompare(b.name));
  const rows = opponents.map((row) =>
    h(
      "tr",
      {},
      h("td", {}, row.name),
      h("td", {}, classText(row)),
      h("td", { class: "num" }, String(row.hands)),
      h("td", { class: "num" }, String(row.actions)),
      h("td", {}, band(row.actions)),
      h("td", {}, h("meter", { min: 0, max: 100, value: row.earned_pct }), ` ${row.earned_pct}%`),
    ),
  );
  return [
    title,
    copy,
    panel(
      "Opponents",
      table(
        `${opponents.length} opponent${opponents.length === 1 ? "" : "s"}, by name`,
        [
          ["Opponent"],
          ["Class"],
          ["Hands seen", "num"],
          ["Actions read", "num"],
          ["Read"],
          ["Earned share"],
        ],
        rows,
      ),
    ),
  ];
}

/** Every bot's dossier, by name, with how many opponents each reads. */
export function dossierIndex(fleet) {
  const bots = [...(fleet?.bots ?? [])].sort((a, b) => a.bot.localeCompare(b.bot));
  return [
    h("h1", {}, "Dossiers"),
    h(
      "p",
      { class: "muted" },
      "Each bot's reads are its own, from its own store. Live opponents only: the Arena's ",
      "archetypes are on the scoreboard.",
    ),
    h(
      "ul",
      {},
      bots.map((status) => {
        const count = Array.isArray(status.opponents) ? status.opponents.length : 0;
        return h(
          "li",
          {},
          h("a", { href: `/bot/${status.bot}/dossier` }, status.bot),
          ` — ${count} opponent${count === 1 ? "" : "s"}`,
        );
      }),
    ),
  ];
}
