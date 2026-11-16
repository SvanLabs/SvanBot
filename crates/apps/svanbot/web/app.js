// The dashboard's page (V11-11): one event stream in, one view out. Everything it shows comes
// through `fleet.js`, and every name on it is set as text, never as markup: an opponent chooses
// their own name.

import {
  botView,
  chart,
  duration,
  fleetProblems,
  freshness,
  isFleet,
  rateText,
  route,
  running,
  winRate,
} from "./fleet.js";

const SVG = "http://www.w3.org/2000/svg";

/** An element with attributes and children. A string child is text. */
function h(tag, attrs = {}, ...children) {
  const svg = ["svg", "polyline", "line", "title"].includes(tag) && attrs.svg !== false;
  const node = svg ? document.createElementNS(SVG, tag) : document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (name !== "svg" && value !== null && value !== undefined) {
      node.setAttribute(name, String(value));
    }
  }
  for (const child of children.flat()) {
    if (child !== null && child !== undefined) {
      node.append(child);
    }
  }
  return node;
}

/** A definition list from pairs of a term and what it is. */
function facts(pairs) {
  return h(
    "dl",
    {},
    pairs.flatMap(([term, value]) => [h("dt", {}, term), h("dd", {}, value)]),
  );
}

function table(caption, heads, rows) {
  return h(
    "table",
    {},
    h("caption", { class: "muted" }, caption),
    h("thead", {}, h("tr", {}, heads.map(([text, cls]) => h("th", { scope: "col", class: cls }, text)))),
    h("tbody", {}, rows),
  );
}

function panel(title, ...children) {
  return h("section", { class: "card" }, h("h2", {}, title), children);
}

function when(ms, nowMs) {
  return ms === null || ms === undefined ? "never" : `${duration(nowMs - ms)} ago`;
}

/** The fleet page: what needs attention, then one card per bot. */
function fleetView(fleet, nowMs, problems) {
  return [
    h("h1", {}, "Fleet"),
    problems.length > 0
      ? h("ul", { class: "problems" }, problems.map((problem) => h("li", { class: problem.tone }, problem.text)))
      : null,
    h(
      "div",
      { class: "grid" },
      fleet.bots.map((status) => {
        const view = botView(status, nowMs);
        return h(
          "article",
          { class: "card" },
          h(
            "header",
            {},
            h("h2", {}, h("a", { href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot)),
            h("span", { class: `pill ${view.tone}` }, view.label),
          ),
          h("p", {}, view.detail),
          facts([
            ["Win rate", rateText(winRate(status.play))],
            ["Hands", `${status.hands} kept, ${status.play.counted} counted`],
            ["Last hand", when(status.last_hand_ms, nowMs)],
            ["Incidents", String(status.incidents)],
          ]),
        );
      }),
    ),
  ];
}

/** The line chart of a bot's latest hands, with its numbers in a table under it. */
function ledgerChart(play) {
  const totals = running(play.recent);
  if (totals.length === 0) {
    return h("p", { class: "muted" }, "No counted hands yet.");
  }
  const [width, height] = [300, 80];
  const { points, zero } = chart(totals, width, height);
  const last = totals[totals.length - 1];
  const label = `Running total over the latest ${totals.length} counted hands: ${last.toFixed(1)} big blinds, between ${Math.min(0, ...totals).toFixed(1)} and ${Math.max(0, ...totals).toFixed(1)}.`;
  return [
    h(
      "svg",
      { class: "chart", viewBox: `0 0 ${width} ${height}`, preserveAspectRatio: "none", role: "img", "aria-label": label },
      h("title", {}, label),
      h("line", { x1: 0, x2: width, y1: zero, y2: zero }),
      h("polyline", { points }),
    ),
    h(
      "details",
      {},
      h("summary", {}, "The chart as a table"),
      h(
        "div",
        { class: "scroll", tabindex: 0 },
        table(
          "Latest counted hands, oldest first, luck of our own draws taken out",
          [["Hand", "number"], ["Won (bb)", "number"], ["Running (bb)", "number"]],
          play.recent.map((mbb, at) =>
            h(
              "tr",
              {},
              h("td", { class: "number" }, String(at + 1)),
              h("td", { class: "number" }, (mbb / 1_000).toFixed(2)),
              h("td", { class: "number" }, totals[at].toFixed(2)),
            ),
          ),
        ),
      ),
    ),
  ];
}

/** One bot's page: six panels at most. */
function botPage(fleet, name, nowMs) {
  const status = fleet.bots.find((bot) => bot.bot === name);
  if (!status) {
    return [h("h1", {}, name), h("p", {}, "This bot has written no status.")];
  }
  const view = botView(status, nowMs);
  const seats = status.table;
  return [
    h("h1", {}, status.bot, " ", h("span", { class: `pill ${view.tone}` }, view.label)),
    h(
      "div",
      { class: "grid" },
      panel(
        "Now",
        h("p", {}, view.detail),
        facts([
          ["Status written", when(status.updated_ms, nowMs)],
          ["Last hand", when(status.last_hand_ms, nowMs)],
          ["Last error", status.last_error ?? "none"],
          ["Incidents", String(status.incidents)],
        ]),
      ),
      panel(
        "Table",
        seats === null
          ? h("p", { class: "muted" }, "Not at a table.")
          : [
              facts([
                ["Table", seats.id],
                ["Street", seats.street ?? "between hands"],
                ["Board", seats.board.length > 0 ? seats.board.join(" ") : "none"],
                ["Pot", String(seats.pot)],
              ]),
              table(
                "Seats",
                [["Seat", "number"], ["Player", null], ["Stack", "number"], ["", null]],
                seats.players.map((seat) =>
                  h(
                    "tr",
                    { class: seat.folded ? "folded" : null },
                    h("td", { class: "number" }, String(seat.seat)),
                    h("td", {}, seat.name, seat.seat === seats.seat ? " (us)" : ""),
                    h("td", { class: "number" }, String(seat.stack)),
                    h("td", {}, seat.folded ? "folded" : seat.seat === seats.dealer ? "button" : ""),
                  ),
                ),
              ),
            ],
      ),
      panel(
        "Ledger",
        facts([
          ["Win rate", rateText(winRate(status.play))],
          ["Counted hands", String(status.play.counted)],
          ["Kept hands", String(status.hands)],
        ]),
        ledgerChart(status.play),
      ),
      panel(
        "Plays by",
        facts([
          ["Champion", h("code", {}, status.play.champion || "unknown")],
          ["Probe", status.play.probe ?? "none running"],
        ]),
      ),
      panel(
        "Process",
        facts([
          ["Build", status.version || "a checkout"],
          ["Started", when(status.started_ms, nowMs)],
          ["Process", String(status.pid)],
        ]),
      ),
    ),
  ];
}

/** The release page: what is installed, where the slots point, and what each bot runs. */
function releasePage(fleet, nowMs) {
  const release = fleet.release ?? { current: null, previous: null, installed: [] };
  return [
    h("h1", {}, "Release"),
    h(
      "div",
      { class: "grid" },
      panel(
        "Slots",
        facts([
          ["Current", release.current ?? "not set"],
          ["Previous", release.previous ?? "not set"],
          ["Installed", release.installed.length > 0 ? release.installed.join(", ") : "nothing"],
        ]),
      ),
      panel(
        "What each bot runs",
        table(
          "Builds and champions by bot",
          [["Bot", null], ["Build", null], ["Started", null], ["Champion", null]],
          fleet.bots.map((status) =>
            h(
              "tr",
              {},
              h("td", {}, h("a", { href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot)),
              h("td", {}, status.version || "a checkout"),
              h("td", {}, when(status.started_ms, nowMs)),
              h("td", { class: "mono" }, status.play.champion.slice(0, 8) || "unknown"),
            ),
          ),
        ),
      ),
    ),
  ];
}

const state = { fleet: null, offset: 0, lastMessageMs: null, streamError: null };

function render() {
  const local = Date.now();
  const fresh = freshness(state.lastMessageMs, local);
  const pill = document.getElementById("fresh");
  pill.className = `pill ${fresh.tone}`;
  pill.textContent = fresh.label;
  if (state.fleet === null) {
    return;
  }
  // The server's clock ages the bots, so a page on a machine with a wrong clock still reads true.
  const nowMs = local + state.offset;
  const problems = fleetProblems(state.fleet, nowMs, state.streamError);
  const count = state.fleet.bots.length;
  document.getElementById("count").textContent = `${count} bot${count === 1 ? "" : "s"}`;
  const worst = document.getElementById("worst");
  worst.className = `worst ${problems[0]?.tone ?? "ok"}`;
  worst.textContent =
    problems.length === 0
      ? "Nothing is wrong."
      : problems.length === 1
        ? problems[0].text
        : `${problems[0].text} (and ${problems.length - 1} more)`;
  const at = route(location.pathname);
  const view =
    at.view === "bot"
      ? botPage(state.fleet, at.bot, nowMs)
      : at.view === "release"
        ? releasePage(state.fleet, nowMs)
        : fleetView(state.fleet, nowMs, problems);
  // Redraw only what changed: a redraw every second would take the focus and the scroll from
  // whoever is reading the table. An open table fallback stays open across a redraw.
  const shown = document.getElementById("view");
  const next = h("main", {}, view);
  const details = next.querySelector("details");
  if (details) {
    details.open = shown.querySelector("details")?.open ?? false;
  }
  if (next.innerHTML !== shown.innerHTML) {
    shown.replaceChildren(...next.childNodes);
  }
}

function take(text) {
  const message = JSON.parse(text);
  state.lastMessageMs = Date.now();
  if (Number.isFinite(message.server_ms)) {
    state.offset = message.server_ms - state.lastMessageMs;
  }
  if (isFleet(message)) {
    state.fleet = message;
  }
  state.streamError = null;
  render();
}

const stream = new EventSource("/api/events");
stream.addEventListener("fleet", (event) => take(event.data));
stream.addEventListener("tick", (event) => take(event.data));
stream.addEventListener("error", () => {
  state.streamError = "The event stream dropped; showing the last fleet received";
  render();
});
setInterval(render, 1_000);
