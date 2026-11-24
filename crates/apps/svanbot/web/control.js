// The control room's two boards (V11-44), after v10's: the fleet room, every bot's table live
// beside the season and the fleet race, and a bot's room, its table large in the middle with its
// ledger, reads, standing and log round it. Each is a board of panels the reader arranges and
// views a slice of at a time (`board.js`).

import { board, viewTabs } from "./board.js";
import { facts, h, table } from "./dom.js";
import { area, chips, felt, summary, tiles } from "./felt.js";
import { botView, chart, duration, rateText, running, winRate } from "./fleet.js";
import { fleetRace, pageHead, rail, tableCard } from "./room.js";
import {
  activityView,
  arenaSummary,
  highlightsView,
  intel,
  notes,
  searchView,
  seasonRace,
  standing,
} from "./widgets.js";

export function when(ms, nowMs) {
  return ms === null || ms === undefined ? "never" : `${duration(nowMs - ms)} ago`;
}

/** The fleet room's views and the panels each shows; All shows every panel. */
export const FLEET_VIEWS = [
  { id: "live", label: "Live", widgets: ["tables", "race", "season", "activity", "notes"] },
  { id: "results", label: "Results", widgets: ["season", "race", "form", "arena", "search"] },
  { id: "opponents", label: "Opponents", widgets: ["reads", "tables"] },
  { id: "system", label: "System", widgets: ["system", "search", "activity", "notes"] },
  { id: "all", label: "All", widgets: null },
];

export const FLEET_LAYOUT = {
  left: ["race", "form", "reads", "notes"],
  center: ["tables", "arena", "search"],
  right: ["season", "activity", "system"],
  hidden: [],
};

/** A bot room's views; All shows every panel. */
export const BOT_VIEWS = [
  {
    id: "live",
    label: "Live",
    widgets: ["table", "now", "highlights", "ledger", "standing", "activity", "notes", "traced"],
  },
  { id: "opponents", label: "Opponents", widgets: ["intel", "table", "notes"] },
  { id: "results", label: "Results", widgets: ["standing", "season", "ledger", "highlights"] },
  { id: "learning", label: "Learning", widgets: ["plays", "arena", "highlights"] },
  { id: "system", label: "System", widgets: ["now", "process", "traced", "activity"] },
  { id: "all", label: "All", widgets: null },
];

export const BOT_LAYOUT = {
  left: ["now", "highlights", "standing", "plays", "intel"],
  center: ["table", "ledger", "traced"],
  right: ["activity", "notes", "season", "process", "arena"],
  hidden: [],
};

/** The page's tools: the table's look, arranging, folding and a reset. */
function tools(ui) {
  return h(
    "div",
    { class: "tools" },
    h(
      "div",
      { class: "segmented", role: "group", "aria-label": "Table look" },
      ["arena", "felt", "midnight"].map((theme) =>
        h(
          "button",
          {
            type: "button",
            "data-act": "theme",
            "data-theme": theme,
            "aria-pressed": String(ui.theme === theme),
          },
          theme,
        ),
      ),
    ),
    h(
      "button",
      { type: "button", "data-act": "arrange", "aria-pressed": String(ui.arranging) },
      ui.arranging ? "Done arranging" : "Arrange",
    ),
    h(
      "button",
      { type: "button", "data-act": ui.collapsed.length > 0 ? "unfold-all" : "fold-all" },
      ui.collapsed.length > 0 ? "Open all" : "Fold all",
    ),
    ui.arranging ? h("button", { type: "button", "data-act": "reset" }, "Reset layout") : null,
  );
}

/** What needs attention, worst first, or nothing. */
function problemList(problems) {
  return problems.length > 0
    ? h(
        "ul",
        { class: "problems" },
        problems.map((problem) => h("li", { class: problem.tone }, problem.text)),
      )
    : null;
}

/** Each bot's form at a glance: its streak, its latest hands and its best and worst. */
function fleetForm(fleet) {
  return table(
    "Each bot over its latest counted hands",
    [
      ["Bot", null],
      ["Win rate", "number"],
      ["Counted", "number"],
      ["Kept", "number"],
    ],
    fleet.bots.map((status) => {
      const rate = winRate(status.play);
      return h(
        "tr",
        {},
        h("td", {}, h("a", { href: `/bot/${encodeURIComponent(status.bot)}` }, status.bot)),
        h("td", { class: `number ${(rate ?? 0) >= 0 ? "up" : "down"}` }, rateText(rate)),
        h("td", { class: "number" }, chips(status.play.counted)),
        h("td", { class: "number" }, chips(status.hands)),
      );
    }),
  );
}

/** Each bot's reads at a glance, the dossier a link away: never pooled across bots. */
function fleetReads(fleet) {
  return [
    h(
      "p",
      { class: "muted" },
      "Each bot reads its opponents on its own, from its own store; no read is shared.",
    ),
    table(
      "Opponents each bot reads",
      [
        ["Bot", null],
        ["Opponents", "number"],
        ["Fixed", "number"],
        ["Switching", "number"],
        ["Reads", null],
      ],
      fleet.bots.map((status) => {
        const rows = status.opponents ?? [];
        const count = (name) => rows.filter((row) => row.class === name).length;
        return h(
          "tr",
          {},
          h(
            "td",
            {},
            h("a", { href: `/bot/${encodeURIComponent(status.bot)}/dossier` }, status.bot),
          ),
          h("td", { class: "number" }, String(rows.length)),
          h("td", { class: "number" }, String(count("fixed"))),
          h("td", { class: "number" }, String(count("switching"))),
          h("td", {}, status.reads_in_play ? "in play" : "held"),
        );
      }),
    ),
  ];
}

/** What each bot runs, and where the install's slots point. */
function system(fleet, nowMs) {
  const release = fleet.release ?? { current: null, previous: null, installed: [] };
  return [
    facts([
      ["Current", release.current ?? "not set"],
      ["Previous", release.previous ?? "not set"],
    ]),
    table(
      "Builds and champions by bot",
      [
        ["Bot", null],
        ["Build", null],
        ["Up", null],
        ["Champion", null],
      ],
      fleet.bots.map((status) =>
        h(
          "tr",
          {},
          h("td", {}, status.bot),
          h("td", {}, status.version || "a checkout"),
          h("td", {}, when(status.started_ms, nowMs)),
          h("td", { class: "mono" }, status.play.champion.slice(0, 8) || "unknown"),
        ),
      ),
    ),
    h("p", {}, h("a", { href: "/release" }, "The release page")),
  ];
}

/** The fleet room: tiles and the rail, then the board. */
export function fleetRoom({ fleet, nowMs, problems, log, ui }) {
  const online = fleet.bots.filter((bot) => bot.connected).length;
  const builds = [...new Set(fleet.bots.map((bot) => bot.version).filter(Boolean))];
  const champions = [...new Set(fleet.bots.map((bot) => bot.play?.champion).filter(Boolean))];
  const reading = fleet.bots.filter((bot) => bot.reads_in_play).length;
  const incidents = fleet.bots.reduce((sum, bot) => sum + (bot.incidents ?? 0), 0);
  const ours = (fleet.season?.rows ?? []).filter((row) => row.ours);
  const best = ours[0];
  const widgets = [
    {
      id: "tables",
      title: "Live tables",
      body: () =>
        h(
          "div",
          { class: "tables" },
          fleet.bots.map((status) => tableCard(status, nowMs, ui.theme)),
        ),
    },
    {
      id: "race",
      title: "Fleet race",
      body: () => [
        h("p", { class: "subtle" }, "Win rate over each bot's counted hands, best first"),
        fleetRace(fleet),
      ],
    },
    { id: "season", title: "Season race", body: () => seasonRace(fleet.season, nowMs) },
    { id: "activity", title: "Activity", body: () => activityView(log, null) },
    { id: "notes", title: "Notes", body: () => notes("fleet", ui.note("fleet")) },
    { id: "form", title: "Form", body: () => fleetForm(fleet) },
    { id: "arena", title: "Arena", body: () => arenaSummary(fleet.arena) },
    { id: "search", title: "Search", body: () => searchView(fleet.search, nowMs) },
    { id: "reads", title: "Reads", body: () => fleetReads(fleet) },
    { id: "system", title: "System", body: () => system(fleet, nowMs) },
  ];
  const view = FLEET_VIEWS.find((one) => one.id === ui.view) ?? FLEET_VIEWS[0];
  return [
    pageHead({
      crumb: "Fleet overview",
      title: "Fleet",
      after: h(
        "span",
        { class: `pill ${online === fleet.bots.length ? "ok" : "warn"}` },
        `${online} of ${fleet.bots.length} online`,
      ),
      detail: "Every bot's table live, the season race, and what changed while you watched.",
      tools: tools(ui),
      nowMs,
    }),
    summary(fleet),
    rail("SV11 signal", "What the fleet plays by, as each bot reports it", [
      ["Build", builds.join(", ") || "—", builds.length > 1 ? "mixed builds" : "one build"],
      [
        "Champion",
        champions.map((digest) => digest.slice(0, 8)).join(", ") || "—",
        "the parameter set in play",
      ],
      [
        "Season",
        best ? `#${best.rank} ${best.name}` : "—",
        fleet.season?.number ? `best of ours, season ${fleet.season.number}` : "not read yet",
      ],
      ["Reads in play", `${reading} of ${fleet.bots.length}`, "per-opponent reads, per bot"],
      ["Incidents", String(incidents), incidents === 0 ? "none recorded" : "see each bot"],
    ]),
    problemList(problems),
    viewTabs(FLEET_VIEWS, view.id),
    board(widgets, view, ui),
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
      h("polygon", { class: "fill", points: area(points, zero) }),
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

/** The newest hands this bot traced, each linking to its replay. */
function tracedHands(bot, hands, nowMs) {
  if (hands === null) {
    return h("p", { class: "muted" }, "Reading the store…");
  }
  if (hands.error) {
    return h("p", { class: "worst fail" }, hands.error);
  }
  if (hands.length === 0) {
    return h("p", { class: "muted" }, "No traced hands yet.");
  }
  return h(
    "div",
    { class: "scroll", tabindex: 0 },
    table(
      "Newest first: open one to replay it",
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
          h(
            "td",
            { class: "mono" },
            h(
              "a",
              { href: `/bot/${bot}/hand/${hand.hand_id}`, title: hand.hand_id },
              String(hand.hand_id).slice(0, 8),
            ),
          ),
          h("td", { class: "mono" }, String(hand.table_id).slice(0, 8)),
          h("td", { class: "number" }, String(hand.decisions)),
          h("td", {}, when(hand.recorded_ms, nowMs)),
        ),
      ),
    ),
  );
}

/** The live table panel: the table drawn, its facts, and its seats as a table. */
function livetable(status, ui) {
  const seats = status.table;
  if (seats === null) {
    return h("div", { class: "felt-empty" }, status.connected ? "Between tables" : "Offline");
  }
  return [
    felt(seats, { opponents: status.opponents ?? [], theme: ui.theme }),
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
          h("td", {}, seat.folded ? "folded" : seat.seat === seats.dealer ? "button" : ""),
        ),
      ),
    ),
  ];
}

/** One bot's room: its tiles and rail, then the board. */
export function botRoom({ fleet, nowMs, log, hands, ui }, name) {
  const status = fleet.bots.find((bot) => bot.bot === name);
  if (!status) {
    return [
      pageHead({ crumb: name, title: name, detail: "This bot has written no status.", nowMs }),
    ];
  }
  const view = botView(status, nowMs);
  const seats = status.table;
  const ours = seats?.players.find((player) => player.seat === seats.seat);
  const rate = winRate(status.play);
  const placed = (fleet.season?.rows ?? []).find((row) => row.name === status.bot);
  const widgets = [
    { id: "table", title: "Table", body: () => livetable(status, ui) },
    {
      id: "now",
      title: "Now",
      body: () => [
        h("p", {}, view.detail),
        facts([
          ["Status written", when(status.updated_ms, nowMs)],
          ["Last hand", when(status.last_hand_ms, nowMs)],
          ["Last error", status.last_error ?? "none"],
          ["Incidents", `${status.run_incidents} this run, ${status.incidents} in all`],
        ]),
      ],
    },
    {
      id: "ledger",
      title: "Ledger",
      body: () => [
        facts([
          ["Win rate", rateText(rate)],
          ["Counted hands", String(status.play.counted)],
          ["Kept hands", String(status.hands)],
        ]),
        ledgerChart(status.play),
      ],
    },
    { id: "highlights", title: "Highlights", body: () => highlightsView(status.play) },
    { id: "standing", title: "Season standing", body: () => standing(fleet.season, status.bot) },
    { id: "season", title: "Season race", body: () => seasonRace(fleet.season, nowMs) },
    { id: "activity", title: "Activity", body: () => activityView(log, status.bot) },
    { id: "notes", title: "Notes", body: () => notes(status.bot, ui.note(status.bot)) },
    { id: "intel", title: "Opponent intel", body: () => intel(status) },
    {
      id: "plays",
      title: "Plays by",
      body: () =>
        facts([
          ["Champion", h("code", {}, status.play.champion || "unknown")],
          ["Probe", status.play.probe ?? "none running"],
          ["Reads", status.reads_in_play ? "in play" : "held"],
        ]),
    },
    { id: "arena", title: "Arena", body: () => arenaSummary(fleet.arena) },
    {
      id: "process",
      title: "Process",
      body: () =>
        facts([
          ["Build", status.version || "a checkout"],
          ["Started", when(status.started_ms, nowMs)],
          ["Process", String(status.pid)],
        ]),
    },
    { id: "traced", title: "Traced hands", body: () => tracedHands(name, hands, nowMs) },
  ];
  const chosen = BOT_VIEWS.find((one) => one.id === ui.view) ?? BOT_VIEWS[0];
  return [
    pageHead({
      crumb: status.bot,
      title: status.bot,
      after: h("span", { class: `pill ${view.tone}` }, view.label),
      detail: [
        view.detail,
        " · ",
        h("a", { href: `/bot/${encodeURIComponent(status.bot)}/dossier` }, "Dossier"),
      ],
      tools: tools(ui),
      nowMs,
    }),
    tiles([
      [
        "Table stack",
        ours ? chips(ours.stack) : "—",
        ours && seats.big_blind > 0
          ? `${Number((ours.stack / seats.big_blind).toFixed(1))} BB behind`
          : "not seated",
        "",
        "chips",
      ],
      [
        "Win rate",
        rateText(rate),
        `over ${chips(status.play.counted)} counted hands`,
        rate === null ? "" : rate >= 0 ? "up" : "down",
        "trend",
      ],
      [
        "Season rank",
        placed ? `#${placed.rank}` : "—",
        placed ? `${chips(placed.score)} points` : "not on the board",
        "",
        "trophy",
      ],
      [
        "Hands played",
        chips(status.hands),
        `${status.run_incidents} incidents this run`,
        "",
        "cards",
      ],
    ]),
    rail("SV11 signal", "What this bot plays by, as it reports it", [
      ["Street", seats?.street ?? "between hands", seats ? `pot ${chips(seats.pot)}` : "no table"],
      ["Build", status.version || "a checkout", `up ${when(status.started_ms, nowMs)}`],
      [
        "Champion",
        (status.play.champion || "unknown").slice(0, 8),
        status.play.probe ?? "no probe running",
      ],
      [
        "Reads",
        status.reads_in_play ? "in play" : "held",
        `${(status.opponents ?? []).length} opponents read`,
      ],
    ]),
    viewTabs(BOT_VIEWS, chosen.id),
    board(widgets, chosen, ui),
  ];
}
