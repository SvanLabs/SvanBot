// The console's live pieces (V11-44): a bot's table drawn on the felt, the fleet's summary bar,
// and a sparkline of a bot's latest hands. Every name is written as text, never as markup.

import { h } from "./dom.js";
import { chart, rateText, running, winRate } from "./fleet.js";

const SUITS = { h: ["♥", true], d: ["♦", true], c: ["♣", false], s: ["♠", false] };

/** One playing card from the venue's notation (`Ah`, `Td`): rank over suit, red suits in red. */
export function playingCard(code) {
  const [symbol, red] = SUITS[code.slice(-1)] ?? ["?", false];
  return h("span", { class: red ? "pcard red" : "pcard" }, `${code.slice(0, -1)}${symbol}`);
}

/** Chips as text, with thousands separated. */
function chips(amount) {
  return Number(amount).toLocaleString("en-US");
}

/**
 * A bot's table on the felt: a seat a player round an oval, our seat at the foot and marked,
 * the folded dimmed, the dealer's button with its seat, and the board and the pot in the middle.
 */
export function felt(table) {
  const players = [...table.players].sort((a, b) => a.seat - b.seat);
  const ours = Math.max(
    0,
    players.findIndex((player) => player.seat === table.seat),
  );
  const seats = players.map((player, at) => {
    // Round the oval from the foot, so our seat is always at the bottom in the middle.
    const turn = ((at - ours + players.length) % players.length) / players.length;
    const angle = Math.PI / 2 + turn * 2 * Math.PI;
    const left = (50 + 44 * Math.cos(angle)).toFixed(1);
    const top = (50 + 40 * Math.sin(angle)).toFixed(1);
    const kind = ["seat", player.seat === table.seat ? "us" : "", player.folded ? "folded" : ""]
      .filter(Boolean)
      .join(" ");
    return h(
      "div",
      { class: kind, style: `left:${left}%;top:${top}%` },
      player.seat === table.dealer ? h("span", { class: "dealer", title: "dealer" }, "D") : null,
      h("span", { class: "name" }, player.name),
      h("span", { class: "stack num" }, chips(player.stack)),
    );
  });
  return h(
    "div",
    { class: "felt", role: "img", "aria-label": `${table.street}, pot ${table.pot}` },
    h(
      "div",
      { class: "middle" },
      h("div", { class: "board" }, table.board.map(playingCard)),
      h("span", { class: "pot num" }, `pot ${table.pot}`),
    ),
    seats,
  );
}

/** The fleet in one bar: how many bots play, the hands they have kept, and the pooled win rate. */
export function summary(fleet) {
  const bots = fleet.bots;
  const playing = bots.filter((bot) => bot.connected).length;
  const hands = bots.reduce((sum, bot) => sum + (bot.hands ?? 0), 0);
  const pooled = winRate({
    counted: bots.reduce((sum, bot) => sum + (bot.play?.counted ?? 0), 0),
    won_mbb: bots.reduce((sum, bot) => sum + (bot.play?.won_mbb ?? 0), 0),
  });
  const versions = [...new Set(bots.map((bot) => bot.version).filter(Boolean))].join(", ");
  const tile = (label, value, tone = "") =>
    h(
      "div",
      { class: "tile" },
      h("span", { class: "label" }, label),
      h("strong", { class: `big ${tone}` }, value),
    );
  return h(
    "div",
    { class: "summary" },
    tile("Bots", `${playing} of ${bots.length} playing`),
    tile("Hands", `${chips(hands)} hands`),
    tile("Fleet win rate", rateText(pooled), pooled === null ? "" : pooled >= 0 ? "up" : "down"),
    tile("Builds", versions || "—"),
  );
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
    h("line", { x1: 0, x2: width, y1: zero, y2: zero }),
    h("polyline", { points }),
  );
}
