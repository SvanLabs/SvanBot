// The dashboard's page (V11-11): one event stream in, one view out. Everything it shows comes
// through `fleet.js`, and every name on it is set as text, never as markup: an opponent chooses
// their own name.

import { facts, h, panel, table } from "./dom.js";
import {
  arenaRun,
  arenaSpan,
  botView,
  byWinRate,
  chart,
  duration,
  fleetProblems,
  freshness,
  isFleet,
  mbbText,
  rateText,
  route,
  running,
  whisker,
  winRate,
} from "./fleet.js";
import { handPage } from "./replay.js";

function when(ms, nowMs) {
  return ms === null || ms === undefined ? "never" : `${duration(nowMs - ms)} ago`;
}

/** The fleet page: what needs attention, then one card per bot. */
function fleetView(fleet, nowMs, problems) {
  return [
    h("h1", {}, "Fleet"),
    problems.length > 0
      ? h(
          "ul",
          { class: "problems" },
          problems.map((problem) => h("li", { class: problem.tone }, problem.text)),
        )
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
            ["Incidents", `${status.run_incidents} this run, ${status.incidents} in all`],
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
      {
        class: "chart",
        viewBox: `0 0 ${width} ${height}`,
        preserveAspectRatio: "none",
        role: "img",
        "aria-label": label,
      },
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
          [
            ["Hand", "number"],
            ["Won (bb)", "number"],
            ["Running (bb)", "number"],
          ],
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
          ["Incidents", `${status.run_incidents} this run, ${status.incidents} in all`],
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
                [
                  ["Seat", "number"],
                  ["Player", null],
                  ["Stack", "number"],
                  ["", null],
                ],
                seats.players.map((seat) =>
                  h(
                    "tr",
                    { class: seat.folded ? "folded" : null },
                    h("td", { class: "number" }, String(seat.seat)),
                    h("td", {}, seat.name, seat.seat === seats.seat ? " (us)" : ""),
                    h("td", { class: "number" }, String(seat.stack)),
                    h(
                      "td",
                      {},
                      seat.folded ? "folded" : seat.seat === seats.dealer ? "button" : "",
                    ),
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
      panel("Traced hands", tracedHands(name)),
    ),
  ];
}

/** The newest hands this bot traced, each linking to its own page. */
function tracedHands(bot) {
  const hands = state.hands;
  if (hands === null) {
    return h("p", { class: "muted" }, "Reading the store…");
  }
  if (hands.error) {
    return h("p", { class: "worst fail" }, hands.error);
  }
  if (hands.length === 0) {
    return h("p", { class: "muted" }, "No traced hands yet.");
  }
  const nowMs = Date.now();
  return table(
    "Newest first",
    [
      ["Hand", null],
      ["Table", null],
      ["Decisions", "number"],
      ["Recorded", null],
    ],
    hands.map((hand) =>
      h(
        "tr",
        {},
        h("td", {}, h("a", { href: `/bot/${bot}/hand/${hand.hand_id}` }, hand.hand_id)),
        h("td", {}, hand.table_id),
        h("td", { class: "number" }, String(hand.decisions)),
        h("td", {}, when(hand.recorded_ms, nowMs)),
      ),
    ),
  );
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
          [
            ["Bot", null],
            ["Build", null],
            ["Started", null],
            ["Champion", null],
          ],
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

/** The scoreboard: the Arena's latest run, a whisker an opponent, and each opponent's trend. */
function scoreboardPage(fleet) {
  const runs = fleet.arena ?? [];
  const latest = arenaRun(runs);
  if (latest === null) {
    return [
      h("h1", {}, "Scoreboard"),
      h(
        "p",
        {},
        "The Arena has not played yet. Record a run with ",
        h("code", {}, "svanbot arena <bouts>"),
        ".",
      ),
    ];
  }
  const span = arenaSpan(latest.rows);
  // A trend is across commits, so its columns are every opponent any run played: taking them
  // from the newest run alone would drop an opponent it skipped from every row.
  const opponents = [...new Set(runs.flatMap((run) => run.rows.map((row) => row.opponent)))].sort(
    (a, b) => a.localeCompare(b),
  );
  const [width, height] = [300, 16];
  return [
    h(
      "h1",
      {},
      "Scoreboard ",
      h(
        "span",
        { class: "muted mono" },
        latest.commit.slice(0, 7),
        " · champion ",
        latest.champion.slice(0, 8),
        " · seed ",
        latest.seed,
      ),
    ),
    h(
      "div",
      { class: "grid" },
      panel(
        "Latest run",
        h(
          "p",
          { class: "muted" },
          `Milli-big-blinds a hundred deals, luck of the cards taken out by playing each deal ` +
            `from both seats. Each whisker is the run's interval, its dark mark the mean, and ` +
            `the dashed line zero. The box spans ${mbbText(span.low)} to ${mbbText(span.high)}. ` +
            `Heads up only: the champion against the pricer and the sparring archetypes, not ` +
            `the wider league.`,
        ),
        table(
          "Each opponent, its whisker, its mean and its interval",
          [
            ["Opponent", null],
            ["Interval", null],
            ["Mean", "number"],
            ["Deals", "number"],
          ],
          byWinRate(latest.rows).map((row) => {
            const bar = whisker(row.mbb_per_100, row.ci95, span, width);
            const label = `${row.opponent}: ${mbbText(row.mbb_per_100)} over ${row.deals} deals, between ${mbbText(row.mbb_per_100 - row.ci95)} and ${mbbText(row.mbb_per_100 + row.ci95)}`;
            return h(
              "tr",
              {},
              h("td", {}, row.opponent),
              h(
                "td",
                {},
                h(
                  "svg",
                  {
                    class: "whisker",
                    viewBox: `0 0 ${width} ${height}`,
                    preserveAspectRatio: "none",
                    role: "img",
                    "aria-label": label,
                  },
                  h("title", {}, label),
                  h("line", { x1: bar.zero, x2: bar.zero, y1: 0, y2: height }),
                  h("line", {
                    x1: bar.from,
                    x2: bar.to,
                    y1: height / 2,
                    y2: height / 2,
                    class: "bar",
                  }),
                  h("line", {
                    x1: bar.mean,
                    x2: bar.mean,
                    y1: 0,
                    y2: height,
                    class: "mean",
                  }),
                ),
              ),
              h("td", { class: "number" }, mbbText(row.mbb_per_100)),
              h("td", { class: "number" }, String(row.deals)),
            );
          }),
        ),
      ),
      panel(
        "Trend",
        h("p", { class: "muted" }, "Each opponent's mean over the recorded runs, newest first."),
        h(
          "div",
          { class: "scroll", tabindex: 0 },
          table(
            "Each run's mean against each opponent",
            [["Commit", null], ...opponents.map((opponent) => [opponent, "number"])],
            runs.map((run) =>
              h(
                "tr",
                {},
                // A promotion moves play under one commit, so the commit alone does not name a
                // run: the champion is what makes two runs of one commit two labels.
                h(
                  "td",
                  { class: "mono" },
                  `${run.commit.slice(0, 7)} · ${run.champion.slice(0, 8)}`,
                ),
                opponents.map((opponent) => {
                  const found = run.rows.find((row) => row.opponent === opponent);
                  return h("td", { class: "number" }, found ? mbbText(found.mbb_per_100) : "—");
                }),
              ),
            ),
          ),
        ),
      ),
    ),
  ];
}

const state = {
  fleet: null,
  offset: 0,
  lastMessageMs: null,
  streamError: null,
  // The views the stream does not carry: one bot's hands, and one hand's trace. Fetched when the
  // path asks for them, and refetched when it changes.
  hands: null,
  handsFor: null,
  trace: null,
  traceFor: null,
};

/** The server's sentence for a refusal, or `fallback` when it sent none: a detail is never markup. */
export function textOf(body, fallback) {
  return typeof body?.detail === "string" ? body.detail : fallback;
}

/** Asks for what the path needs, once per path: the stream carries the fleet, not the traces. */
function load(at) {
  if (at.view === "bot" && state.handsFor !== at.bot) {
    state.handsFor = at.bot;
    state.hands = null;
    fetch(`/api/hands/${at.bot}`)
      .then((answer) =>
        // The server says why a 500 happened; a 404 is a name no bot has. Read the detail, and
        // fall back to the plain sentence if the body is not the JSON it should be.
        answer.ok
          ? answer.json()
          : answer.json().then(
              (body) =>
                Promise.reject(new Error(textOf(body, "The bot's store could not be read."))),
              () => Promise.reject(new Error("The bot's store could not be read.")),
            ),
      )
      .then((body) => {
        state.hands = body.hands ?? [];
        render();
      })
      .catch((error) => {
        state.hands = { error: error.message };
        render();
      });
  }
  const key = at.view === "hand" ? `${at.bot}/${at.hand}` : null;
  if (key !== null && state.traceFor !== key) {
    state.traceFor = key;
    state.trace = null;
    fetch(`/api/trace/${at.bot}/${at.hand}`)
      .then((answer) =>
        // A 500 carries why the store could not be read; a 404 is a hand nobody played. The
        // page must not show a broken store as a hand that does not exist.
        answer.ok
          ? answer.json()
          : answer.json().then(
              (body) =>
                Promise.reject(new Error(textOf(body, "This hand is not in the bot's store."))),
              () => Promise.reject(new Error("This hand is not in the bot's store.")),
            ),
      )
      .then((body) => {
        state.trace = body;
        render();
      })
      .catch((error) => {
        state.trace = { error: error.message };
        render();
      });
  }
}

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
  load(at);
  const view =
    at.view === "hand"
      ? handPage(state, at.bot, at.hand)
      : at.view === "bot"
        ? botPage(state.fleet, at.bot, nowMs)
        : at.view === "release"
          ? releasePage(state.fleet, nowMs)
          : at.view === "scoreboard"
            ? scoreboardPage(state.fleet)
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
