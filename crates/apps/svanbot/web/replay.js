// The hand replayer (V11-24): one hand's page, read from the trace the bot wrote rather than from
// a replay of the hand. Its own module because it is its own reason to change: the fleet view and
// the scoreboard do not move when a decision's record does.

import { facts, h, panel } from "./dom.js";
import { byStreet, chipsText, exploitText, priceText } from "./fleet.js";

/**
 * One hand's page: every decision in it, street by street, from the trace the bot wrote rather
 * than from a replay of the hand (V11-24). It is loopback only, like every page here.
 */
export function handPage(state, bot, hand) {
  const title = h("h1", {}, `${bot} `, h("code", {}, hand));
  const trace = state.trace;
  const head = h("p", {}, h("a", { href: `/bot/${bot}` }, "← back to the bot"));
  if (trace === null) {
    return [title, head, h("p", { class: "muted" }, "Reading the trace…")];
  }
  if (trace.error) {
    return [title, head, h("p", { class: "worst fail" }, trace.error)];
  }
  if (!trace.traced) {
    // A hand from before the trace existed is not a hand that made no decisions.
    return [title, head, h("p", { class: "muted" }, trace.note)];
  }
  const streets = byStreet(trace.decisions);
  return [
    title,
    head,
    ...streets.map((street) =>
      panel(streetTitle(street.name), ...street.decisions.map(decisionView)),
    ),
  ];
}

function streetTitle(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** One decision: the cards, which way decided, what it was worth, and the ranges it read. */
export function decisionView(decision) {
  const priced = decision.branch === "priced";
  return h(
    "div",
    { class: "decision" },
    h(
      "p",
      {},
      h("strong", {}, priced ? "Priced" : "Resolved"),
      priced
        ? `: equity ${(decision.equity * 100).toFixed(1)}%, ${priceText(decision.price)}, ${exploitText(decision.exploit)}`
        : `: took ${actionNamed(decision, decision.taken)}`,
    ),
    h("p", { class: "muted" }, cardsText(decision)),
    priced ? null : facts(decision.values.map((value, at) => actionRow(decision, at, value))),
    ...(decision.ranges ?? []).map((range) =>
      facts([[seatLabel(range.seat), range.masses.map((mass) => mass.toFixed(2)).join(" ")]]),
    ),
  );
}

/**
 * The opponent's seat, which is what a reader can find at the table. A range whose spot carried
 * no seat says so: numbering it from its position in the list would name a player the table does
 * not show, which is a wrong label that reads as a right one.
 */
function seatLabel(seat) {
  return typeof seat === "number" ? `Seat ${seat}` : "Seat unknown";
}

/** `fold`, `check`, `call`, `bet 200`: the name the trace kept, or the bare index when none did. */
function actionNamed(decision, at) {
  const name = decision.actions?.[at];
  return name === undefined ? `action ${at + 1} of ${decision.values.length}` : name;
}

/**
 * One action's value as the re-solve reported it. The value is in chips: it is what the last
 * iteration valued the action at for our cards, not a share of the pot and not big blinds.
 */
function actionRow(decision, at, value) {
  return [
    `Action ${at + 1}: ${actionNamed(decision, at)}`,
    `${chipsText(value)}, mixed ${(decision.strategy[at] * 100).toFixed(0)}%`,
  ];
}

/** The hole cards and the board this decision saw, written the way the store keeps them. */
export function cardsText(decision) {
  const hole = cards(decision.hole);
  const board = cards(decision.board);
  return board ? `${hole || "—"} · board ${board}` : `${hole || "—"} · preflop`;
}

/** Cards written pair by pair, as `Ah Kh`, or "" when there are none. */
function cards(text) {
  if (typeof text !== "string" || text.length === 0) {
    return "";
  }
  const pairs = [];
  for (let at = 0; at + 1 < text.length; at += 2) {
    pairs.push(text.slice(at, at + 2));
  }
  return pairs.join(" ");
}
