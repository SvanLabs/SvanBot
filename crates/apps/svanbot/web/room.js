// The control room's frame (V11-44, V11-45): a chip a bot in the header, the page's head, the
// telemetry rail, a live table a bot, and the fleet race. Every name is text.

import { h } from "./dom.js";
import { chips, felt, sparkline } from "./felt.js";
import { botView, rateText, winRate } from "./fleet.js";
import { icon } from "./icons.js";

/** A bot's dot: playing, between tables, or down. */
function dot(status, nowMs) {
  return h("span", { class: `dot ${botView(status, nowMs).tone}`, "aria-hidden": "true" });
}

/** The top bar's tabs: one a bot, with its dot and its place in the fleet; the bot in view lit. */
export function botTabs(fleet, current, nowMs) {
  return fleet.bots.map((status) =>
    h(
      "a",
      {
        class: "bot-tab",
        href: `/bot/${encodeURIComponent(status.bot)}`,
        "aria-current": status.bot === current ? "page" : null,
      },
      dot(status, nowMs),
      h("span", {}, status.bot),
    ),
  );
}

/**
 * The page's head: where the reader is, the page's title and its state, the page's tools, and
 * when the fleet was last heard.
 */
export function pageHead({ crumb, title, after = null, detail = null, tools = null, nowMs }) {
  const at = new Date(nowMs);
  const clock = [at.getHours(), at.getMinutes(), at.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
  return h(
    "header",
    { class: "page-head" },
    h(
      "div",
      { class: "page-title" },
      h(
        "div",
        { class: "crumbs" },
        h("span", { class: "crumb" }, "Workspace"),
        h("span", { class: "chev", "aria-hidden": "true" }, "/"),
        h("span", { class: "crumb here" }, crumb),
        h("span", { class: "env" }, "openpoker · virtual chips"),
      ),
      h("h1", {}, title, after ? [" ", after] : null),
      detail ? h("p", { class: "status-line" }, detail) : null,
    ),
    h(
      "div",
      { class: "page-tools" },
      tools,
      h("span", { class: "updated" }, h("i", { class: "live-dot" }), `Updated ${clock}`),
    ),
  );
}

/** The telemetry rail: a lead cell, then a term, a value and a note a cell. */
export function rail(lead, note, cells) {
  return h(
    "div",
    { class: "rail-strip" },
    h(
      "div",
      { class: "rail-lead" },
      h("span", { class: "rail-icon" }, icon("pulse")),
      h("b", {}, lead),
      note ? h("small", {}, note) : null,
    ),
    h(
      "dl",
      {},
      cells.map(([term, value, detail]) =>
        h("div", {}, h("dt", {}, term), h("dd", {}, value), detail ? h("small", {}, detail) : null),
      ),
    ),
  );
}

/**
 * One bot's live table, as the fleet overview shows it: a header, the table with what this bot
 * reads of each opponent at it, and a footer.
 */
export function tableCard(status, nowMs, theme = "arena") {
  const view = botView(status, nowMs);
  const rate = winRate(status.play);
  const won = (status.play?.won_mbb ?? 0) / 1_000;
  return h(
    "article",
    { class: "card table-card" },
    h(
      "header",
      {},
      h(
        "h2",
        {},
        dot(status, nowMs),
        h("a", { href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot),
      ),
      h("span", { class: `pill ${view.tone}` }, view.label),
    ),
    h("p", { class: "subtle" }, view.detail),
    h(
      "div",
      { class: "headline" },
      h("strong", { class: `big ${(rate ?? 0) >= 0 ? "up" : "down"}` }, rateText(rate)),
      sparkline(status.play),
    ),
    status.table
      ? felt(status.table, { opponents: status.opponents ?? [], theme })
      : h("div", { class: "felt-empty" }, status.connected ? "Between tables" : "Offline"),
    h(
      "footer",
      {},
      h(
        "span",
        { class: "mono" },
        `${chips(status.hands ?? 0)} hands · build ${status.version ?? "—"}`,
      ),
      h(
        "span",
        { class: `mono ${won >= 0 ? "up" : "down"}` },
        `${won >= 0 ? "+" : ""}${won.toFixed(1)} bb ledger`,
      ),
    ),
  );
}

/** The fleet race: each bot's win rate as a bar from zero, best first, with its latest form. */
export function fleetRace(fleet) {
  const rows = fleet.bots
    .map((status) => ({ status, rate: winRate(status.play) }))
    .sort((a, b) => (b.rate ?? -Infinity) - (a.rate ?? -Infinity));
  const widest = Math.max(1, ...rows.map((row) => Math.abs(row.rate ?? 0)));
  return h(
    "div",
    { class: "race" },
    rows.map(({ status, rate }, place) => {
      const share = ((Math.abs(rate ?? 0) / widest) * 50).toFixed(1);
      const side = (rate ?? 0) >= 0 ? "up" : "down";
      return h(
        "div",
        { class: "race-row" },
        h("span", { class: "place" }, String(place + 1)),
        h("a", { class: "race-name", href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot),
        h(
          "span",
          { class: "race-track", "aria-hidden": "true" },
          h("i", { class: `race-bar ${side}`, style: `--share:${share}%` }),
        ),
        h("span", { class: `race-rate ${side}` }, rateText(rate)),
        sparkline(status.play),
      );
    }),
  );
}
