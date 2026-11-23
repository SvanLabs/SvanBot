// The control room's live pieces (V11-44): a bot's table drawn as an arena table, the fleet's
// summary tiles, and a sparkline of a bot's latest hands. Every name is written as text, never as
// markup: an opponent chooses their own name.

import { h } from "./dom.js";
import { chart, rateText, running, winRate } from "./fleet.js";
import { icon } from "./icons.js";

const SUITS = { h: ["♥", true], d: ["♦", true], c: ["♣", false], s: ["♠", false] };

/** One playing card from the venue's notation (`Ah`, `Td`): rank over suit, red suits in red. */
export function playingCard(code) {
  const [symbol, red] = SUITS[code.slice(-1)] ?? ["?", false];
  const rank = code.slice(0, -1);
  return h("span", { class: red ? "pcard red" : "pcard" }, h("b", {}, rank), h("i", {}, symbol));
}

/** Chips as text, with thousands separated. */
export function chips(amount) {
  return Number(amount).toLocaleString("en-US");
}

/** A player's initials for their seat's badge: the first two letters or digits of their name. */
export function initials(name) {
  const letters = String(name ?? "").replace(/[^\p{L}\p{N}]/gu, "");
  return (letters.slice(0, 2) || "··").toUpperCase();
}

/** The table's looks, as v10 offered them: the arena's charcoal, a green felt, or midnight. */
export const THEMES = ["arena", "felt", "midnight"];

/**
 * A bot's table as an arena table: the rail and the felt, a seat a player round it with their
 * initials, name, stack and whether they are still in, our seat at the foot and lit, card backs
 * in front of everyone still in, the dealer's button, and the board and the pot (in chips and big
 * blinds) in the middle. With `opponents`, the bot's own dossier, each opponent's plate says what
 * the bot reads of them.
 */
export function felt(table, { opponents = null, theme = "arena" } = {}) {
  const reads = opponents === null ? null : new Map(opponents.map((row) => [row.name, row.class]));
  const players = [...table.players].sort((a, b) => a.seat - b.seat);
  const ours = Math.max(
    0,
    players.findIndex((player) => player.seat === table.seat),
  );
  const seats = players.map((player, at) => {
    // Round the table from the foot, so our seat is always at the bottom in the middle.
    const turn = ((at - ours + players.length) % players.length) / players.length;
    const angle = Math.PI / 2 + turn * 2 * Math.PI;
    const left = (50 + 46 * Math.cos(angle)).toFixed(1);
    const top = (50 + 43 * Math.sin(angle)).toFixed(1);
    const us = player.seat === table.seat;
    const kind = ["seat", us ? "us" : "", player.folded ? "folded" : ""].filter(Boolean).join(" ");
    return h(
      "div",
      { class: kind, style: `left:${left}%;top:${top}%` },
      player.folded
        ? null
        : h("span", { class: "backs", "aria-hidden": "true" }, h("i", {}), h("i", {})),
      h(
        "span",
        { class: "plate" },
        h("span", { class: "avatar", "aria-hidden": "true" }, initials(player.name)),
        h(
          "span",
          { class: "who" },
          h("span", { class: "name" }, player.name),
          h("span", { class: "stack", "data-flash": "" }, chips(player.stack)),
        ),
        us ? h("span", { class: "you" }, "you") : null,
        !us && reads !== null
          ? h(
              "span",
              { class: `read-tag ${reads.get(player.name) ?? "none"}` },
              reads.get(player.name) ?? "no read",
            )
          : null,
      ),
      h("span", { class: "state" }, player.folded ? "folded" : "in hand"),
      player.seat === table.dealer ? h("span", { class: "dealer", title: "dealer" }, "D") : null,
    );
  });
  const bb = table.big_blind > 0 ? table.pot / table.big_blind : null;
  return h(
    "div",
    {
      class: `felt theme-${THEMES.includes(theme) ? theme : "arena"}`,
      role: "img",
      "aria-label": `${table.street ?? "between hands"}, pot ${table.pot}`,
    },
    h("div", { class: "rail" }),
    h(
      "div",
      { class: "middle" },
      h("div", { class: "board" }, table.board.map(playingCard)),
      h(
        "div",
        { class: "pot" },
        h("span", { class: "pot-label" }, "pot"),
        h("span", { class: "pot-amount", "data-flash": "" }, chips(table.pot)),
        bb === null ? null : h("span", { class: "pot-bb" }, `${Number(bb.toFixed(1))} BB`),
      ),
    ),
    seats,
  );
}

/** The fleet in tiles: how many bots play, the hands they have kept, and the pooled win rate. */
export function summary(fleet) {
  const bots = fleet.bots;
  const playing = bots.filter((bot) => bot.connected).length;
  const hands = bots.reduce((sum, bot) => sum + (bot.hands ?? 0), 0);
  const counted = bots.reduce((sum, bot) => sum + (bot.play?.counted ?? 0), 0);
  const pooled = winRate({
    counted,
    won_mbb: bots.reduce((sum, bot) => sum + (bot.play?.won_mbb ?? 0), 0),
  });
  const best = [...bots]
    .filter((bot) => winRate(bot.play) !== null)
    .sort((a, b) => winRate(b.play) - winRate(a.play))[0];
  return tiles([
    [
      "Bots online",
      `${playing} of ${bots.length} playing`,
      `${bots.length - playing} offline`,
      "",
      "bots",
    ],
    [
      "Hands played",
      `${chips(hands)} hands`,
      `${chips(counted)} counted for the ledger`,
      "",
      "cards",
    ],
    [
      "Fleet win rate",
      rateText(pooled),
      "pooled over every counted hand",
      pooled === null ? "" : pooled >= 0 ? "up" : "down",
      "trend",
    ],
    [
      "Leading bot",
      best ? best.bot : "—",
      best ? rateText(winRate(best.play)) : "no counted hands yet",
      "",
      "trophy",
    ],
  ]);
}

/**
 * Hero tiles: an icon, a label, a big value and a note each, the value toned up or down when
 * given, and flashing once when it changes.
 */
export function tiles(rows) {
  return h(
    "div",
    { class: "summary" },
    rows.map(([label, value, note, tone = "", glyph = "bolt"]) =>
      h(
        "div",
        { class: `tile${tone ? ` tone-${tone}` : ""}` },
        h("span", { class: "tile-icon" }, icon(glyph)),
        h("span", { class: "label" }, label),
        h("strong", { class: `big ${tone}`.trim(), "data-flash": "" }, value),
        note ? h("small", {}, note) : null,
      ),
    ),
  );
}

/** The area between a chart's line and its zero rule, for the soft fill under the line. */
export function area(points, zero) {
  const xs = points === "" ? [] : points.split(" ").map((point) => point.split(",")[0]);
  return xs.length === 0 ? "" : `${xs[0]},${zero} ${points} ${xs.at(-1)},${zero}`;
}

/** A bot's running total over its latest hands, as a small line. */
export function sparkline(play) {
  const totals = running(play?.recent ?? []);
  const [width, height] = [120, 28];
  const { points, zero } = chart(totals, width, height);
  const last = totals.at(-1) ?? 0;
  return h(
    "svg",
    {
      class: `spark ${last >= 0 ? "up" : "down"}`,
      viewBox: `0 0 ${width} ${height}`,
      preserveAspectRatio: "none",
      role: "img",
      "aria-label": `Running total over the latest ${totals.length} hands: ${last.toFixed(1)} big blinds`,
    },
    h("polygon", { class: "fill", points: area(points, zero) }),
    h("line", { x1: 0, x2: width, y1: zero, y2: zero }),
    h("polyline", { points }),
  );
}
