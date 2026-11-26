// The opponent stats (V11-121): one bot's own reads, rebuilt from its own store, shown the way
// they are earned — every rate beside its counts, an interval once the evidence clears the floor,
// and "not enough observations" when it does not. Read-only; the page never writes a store.

import { h, panel } from "./dom.js";

/** The evidence floor: hands at a stat below which no rate is shown. */
const FLOOR = 30;

/** "8 of 40: 20% (12–33%)" — or the floor said out loud, never a bare percentage. */
export function statText(stat) {
  const counts = `${stat.hits} of ${stat.opportunities}`;
  if (stat.floor || stat.rate == null) {
    return `${counts} · not enough observations`;
  }
  const percent = (value) => `${Math.round(value * 100)}%`;
  const interval = Array.isArray(stat.interval)
    ? ` (${percent(stat.interval[0])}–${percent(stat.interval[1])})`
    : "";
  return `${counts} · ${percent(stat.rate)}${interval}`;
}

/** The situations a read actually has evidence in, with thin cells flagged. */
function evidenceLine(situations) {
  const seen = (situations ?? []).filter((cell) => cell.n > 0);
  if (seen.length === 0) {
    return "No situation counts yet.";
  }
  return seen
    .map(
      (cell) =>
        `${cell.street}${cell.facing ? " facing a bet" : ""}: ${cell.n}${
          cell.sparse ? " (thin)" : ""
        }`,
    )
    .join(" · ");
}

function opponentCard(bot, opponent) {
  const stats = opponent.stats ?? {};
  const row = (label, stat) =>
    h("tr", {}, h("td", {}, label), h("td", { class: "num" }, statText(stat)));
  const hands = (opponent.hand_ids ?? []).map((id) =>
    h("li", {}, h("a", { href: `/bot/${bot}/hand/${id}` }, id)),
  );
  return h(
    "details",
    { class: "reads-opponent" },
    h(
      "summary",
      {},
      `${opponent.name} — ${opponent.hands} hands · ${opponent.class} · earned share ${Math.round(
        (opponent.earned ?? 0) * 100,
      )}%`,
    ),
    h(
      "table",
      { class: "reads-table" },
      h(
        "tbody",
        {},
        row("VPIP", stats.vpip),
        row("PFR", stats.pfr),
        row("Fold to bet", stats.fold_to_bet),
        row("Aggression", stats.aggression),
      ),
    ),
    h("p", { class: "muted" }, evidenceLine(opponent.situations)),
    hands.length > 0
      ? h(
          "details",
          {},
          h("summary", {}, `${hands.length} hands behind these stats`),
          h("ul", {}, hands),
        )
      : null,
  );
}

/**
 * The stats panel for one bot, drawn from what `app.js` fetched: `undefined` while the store is
 * being read, `{ error }` when it could not be, `{ opponents }` when it could. Pure, so the page
 * tests render it against a stand-in for the browser.
 */
export function readsPanel(bot, reads) {
  const body = h("div", {});
  if (!reads) {
    body.append(h("p", { class: "muted" }, "Reading the store…"));
    return panel("Opponent stats", body);
  }
  if (reads.error) {
    body.append(h("p", {}, reads.error));
    return panel("Opponent stats", body);
  }
  const opponents = Array.isArray(reads.opponents) ? reads.opponents : [];
  if (opponents.length === 0) {
    body.append(h("p", {}, "This bot has no reads yet: it has played no named opponent."));
    return panel("Opponent stats", body);
  }
  body.append(
    h(
      "p",
      { class: "muted" },
      `Each rate is shown with its counts; under ${FLOOR} opportunities a stat shows the floor ` +
        "message instead of a number. Intervals are a display approximation, not the bot's own " +
        "gate; reads are per bot, from its own store.",
    ),
    ...opponents.map((opponent) => opponentCard(bot, opponent)),
  );
  return panel("Opponent stats", body);
}
