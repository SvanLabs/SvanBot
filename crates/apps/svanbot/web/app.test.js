// The page's script, run against a stand-in for the browser: enough of a DOM to build the
// views, and an event stream the test feeds by hand. It holds what each view shows and that a
// name is written as text, never as markup.
import { beforeAll, expect, test } from "bun:test";

import { byClass, Node, standIn } from "./testdom.js";

const ids = {
  view: new Node("main"),
  fresh: new Node("span"),
  count: new Node("span"),
  worst: new Node("strong"),
  bots: new Node("nav"),
};
const listeners = {};
const NOW = 1_800_000_000_000;

function fleet() {
  const play = {
    champion: "308e7da4b29fd533",
    probe: null,
    counted: 200,
    won_mbb: 3_000,
    recent: [1_000, -3_000, 500],
  };
  const base = {
    pid: 7,
    version: "11.0.0",
    started_ms: NOW - 3_600_000,
    updated_ms: NOW - 1_000,
    connected: true,
    table: null,
    hands: 210,
    incidents: 0,
    run_incidents: 0,
    last_hand_ms: NOW - 30_000,
    last_error: null,
    play,
  };
  return {
    server_ms: NOW,
    problems: [],
    season: null,
    release: { current: "11.0.0", previous: "10.9.0", installed: ["10.9.0", "11.0.0"] },
    arena: [
      {
        commit: "abc1234def",
        champion: "308e7da4b29fd533",
        seed: 7,
        rows: [
          { opponent: "nit", deals: 400, mbb_per_100: 12_345.6, ci95: 2_000.4 },
          { opponent: "<b>maniac</b>", deals: 400, mbb_per_100: -5_000, ci95: 9_000 },
        ],
      },
      {
        commit: "older999",
        champion: "308e7da4b29fd533",
        seed: 7,
        rows: [{ opponent: "nit", deals: 400, mbb_per_100: 1_000, ci95: 500 }],
      },
    ],
    bots: [
      {
        ...base,
        bot: "SurSvan",
        opponents: [
          {
            name: "someone",
            class: "fixed",
            change: null,
            hands: 90,
            actions: 140,
            earned_pct: 35,
          },
        ],
        table: {
          id: "t-17",
          seat: 2,
          hand_id: "h-1",
          street: "turn",
          board: ["Ah", "Kd", "7c", "2s"],
          pot: 640,
          big_blind: 20,
          dealer: 0,
          players: [
            { seat: 0, name: "<img src=x onerror=alert(1)>", stack: 1810, folded: false },
            { seat: 2, name: "SurSvan", stack: 2240, folded: false },
            { seat: 4, name: "someone", stack: 1950, folded: true },
          ],
        },
      },
      { ...base, bot: "MegaSvan", connected: false, last_error: "socket: reset" },
    ],
  };
}

/** Shows `path` with the fleet just received, and returns the view. */
function show(path) {
  globalThis.location.pathname = path;
  listeners.fleet({ data: JSON.stringify(fleet()) });
  return ids.view;
}

beforeAll(async () => {
  globalThis.document = {
    ...standIn,
    getElementById: (id) => ids[id],
    querySelector: (tag) => ids.view.querySelector(tag),
  };
  globalThis.location = { pathname: "/" };
  globalThis.history = {
    pushState: (_state, _title, url) => {
      globalThis.location.pathname = url;
    },
  };
  globalThis.EventSource = class {
    addEventListener(name, listener) {
      listeners[name] = listener;
    }
  };
  globalThis.setInterval = () => 0;
  Date.now = () => NOW;
  await import("./app.js");
});

test("a refusal's sentence is the server's when it sent one, and the page's when it did not", async () => {
  // The trace route answers 500 with a sentence saying why the store could not be read. The page
  // shows that sentence, so a corrupt store does not read as "no such hand". A detail that is not
  // a string is never shown: it would be an object where a sentence belongs.
  const { textOf } = await import("./app.js");
  expect(textOf({ detail: "Alpha's store cannot be read: no such table" }, "fallback")).toBe(
    "Alpha's store cannot be read: no such table",
  );
  expect(textOf({}, "fallback")).toBe("fallback");
  expect(textOf({ detail: { note: "obj" } }, "fallback")).toBe("fallback");
  expect(textOf(null, "fallback")).toBe("fallback");
});

test("the fleet page has one card a bot and the strip names the worst problem", () => {
  const view = show("/");
  expect(view.all("article").length).toBe(2);
  expect(ids.count.textContent).toBe("2 bots");
  expect(ids.fresh.textContent).toBe("live");
  expect(ids.worst.textContent).toBe("MegaSvan is offline: socket: reset");
  expect(view.textContent).toContain("t-17 · turn · pot 640");
  expect(view.textContent).toContain("+1.5 bb/100");
  expect(view.querySelector("a").attrs.href).toBe("/bot/SurSvan");
});

test("a fleet card draws its live table: seats with plates, our seat, the button, the board, the pot", () => {
  const view = show("/");
  const [felt] = byClass(view, "felt");
  const seats = byClass(felt, "seat");
  expect(seats.length).toBe(3);
  const plate = (seat) => [
    byClass(seat, "name")[0].textContent,
    byClass(seat, "stack")[0].textContent,
  ];
  expect(byClass(felt, "us").map(plate)).toEqual([["SurSvan", "2,240"]]);
  expect(byClass(felt, "folded").map(plate)).toEqual([["someone", "1,950"]]);
  // Everyone still in has card backs in front of them; the folded seat has none.
  expect(seats.map((seat) => byClass(seat, "backs").length)).toEqual([1, 1, 0]);
  expect(byClass(felt, "state").map((tag) => tag.textContent)).toEqual([
    "in hand",
    "in hand",
    "folded",
  ]);
  // The dealer's button sits with seat 0, whose name is text, never markup.
  expect(byClass(seats[0], "dealer").length).toBe(1);
  expect(byClass(seats[0], "name")[0].textContent).toBe("<img src=x onerror=alert(1)>");
  expect(byClass(seats[0], "avatar")[0].textContent).toBe("IM");
  const cards = byClass(felt, "pcard");
  expect(cards.map((card) => card.textContent)).toEqual(["A♥", "K♦", "7♣", "2♠"]);
  expect(byClass(felt, "red").length).toBe(2);
  expect(byClass(felt, "pot-amount")[0].textContent).toBe("640");
  expect(byClass(felt, "pot-bb")[0].textContent).toBe("32 BB");
  // A bot with no table has no felt.
  expect(byClass(view, "felt").length).toBe(1);
});

test("the fleet overview opens with management metrics and a linked roster", () => {
  const view = show("/");
  const [metrics] = byClass(view, "management-metrics");
  expect(metrics.textContent).toContain("1 / 2");
  expect(metrics.textContent).toContain("Needs attention");
  const [roster] = byClass(view, "management-roster");
  expect(roster.all("a").map((anchor) => anchor.attrs.href)).toEqual([
    "/bot/SurSvan",
    "/bot/MegaSvan",
  ]);
  expect(roster.textContent).toContain("Offline");
  expect(view.textContent).toContain("420 kept");
  expect(roster.textContent).toContain("+1.5 bb/100");
  expect(view.all("article").map((card) => byClass(card, "spark").length)).toEqual([1, 1]);
  expect(byClass(view, "race-row").length).toBe(2);
});

test("appearing or clearing a fleet alert preserves the bot status links", () => {
  show("/");
  const healthy = fleet();
  healthy.problems = [];
  healthy.bots = healthy.bots.map((bot) => ({ ...bot, connected: true, run_incidents: 0 }));
  const receive = (problems) => listeners.fleet({ data: JSON.stringify({ ...healthy, problems }) });
  receive([]);
  const roster = () => byClass(ids.view, "management-roster")[0];
  const first = roster().all("a")[0];
  receive(["The Arena run is missing"]);
  expect(roster().all("a")[0]).toBe(first);
  receive([]);
  expect(roster().all("a")[0]).toBe(first);
});

test("the top bar has a tab a bot, and the bot in view is lit", () => {
  show("/bot/SurSvan");
  const tabs = ids.bots.all("a");
  expect(tabs.map((tab) => tab.attrs.href)).toEqual(["/bot/SurSvan", "/bot/MegaSvan"]);
  expect(tabs.map((tab) => tab.attrs["aria-current"] ?? null)).toEqual(["page", null]);
});

/** The titles of the panels on the board, in order, column by column. */
function titles(view) {
  return byClass(view, "widget").map((panel) => panel.querySelector("h2").textContent);
}

test("a bot's room opens on Live, and All shows every panel", async () => {
  const { act } = await import("./app.js");
  const view = show("/bot/SurSvan");
  expect(titles(view)).toEqual([
    "Now",
    "Highlights",
    "Season standing",
    "Table",
    "Ledger",
    "Traced hands",
    "Activity",
    "Notes",
  ]);
  const tabs = byClass(view, "view-tabs")[0].all("button");
  expect(tabs.map((tab) => tab.textContent)).toEqual([
    "Live",
    "Opponents",
    "Results",
    "Learning",
    "System",
    "All",
  ]);
  act("view", { view: "all" });
  expect(titles(view)).toEqual([
    "Now",
    "Highlights",
    "Season standing",
    "Plays by",
    "Opponent intel",
    "Table",
    "Ledger",
    "Traced hands",
    "Activity",
    "Notes",
    "Season race",
    "Process",
    "Arena",
  ]);
  // The reads beside the table they are about.
  act("view", { view: "opponents" });
  expect(titles(view)).toEqual(["Opponent intel", "Table", "Notes"]);
  act("view", { view: "live" });
});

test("the reader arranges, folds and hides panels, and a reset puts the board back", async () => {
  const { act } = await import("./app.js");
  const view = show("/bot/SurSvan");
  act("arrange");
  expect(byClass(view, "move").length).toBeGreaterThan(0);
  act("move", { id: "notes", dir: "up" });
  const right = () => titles(byClass(view, "right")[0]);
  expect(right()).toEqual(["Notes", "Activity"]);
  act("fold", { id: "ledger" });
  const ledger = () =>
    byClass(view, "widget").find((panel) => panel.attrs["data-widget"] === "ledger");
  expect(ledger().attrs["data-collapsed"]).toBe("true");
  expect(byClass(ledger(), "chart").length).toBe(0);
  act("hide", { id: "traced" });
  expect(titles(view)).not.toContain("Traced hands");
  expect(byClass(view, "tray")[0].textContent).toContain("Traced hands");
  act("reset");
  expect(right()).toEqual(["Activity", "Notes"]);
  expect(titles(view)).toContain("Traced hands");
  expect(ledger().attrs["data-collapsed"]).toBe(undefined);
  act("arrange");
  expect(byClass(view, "move").length).toBe(0);
});

test("the felt says what the bot reads of each opponent at its table, in the look chosen", async () => {
  const { act } = await import("./app.js");
  const view = show("/bot/SurSvan");
  const [felt] = byClass(view, "felt");
  expect(felt.attrs.class).toBe("felt theme-arena");
  const tags = byClass(felt, "read-tag").map((tag) => tag.textContent);
  expect(tags).toEqual(["no read", "fixed"]);
  act("theme", { theme: "midnight" });
  expect(byClass(view, "felt")[0].attrs.class).toBe("felt theme-midnight");
  act("theme", { theme: "arena" });
});

test("the season shows on the fleet's board and as each bot's rank", () => {
  const next = fleet();
  next.season = {
    number: 17,
    remaining_s: 7_200,
    read_ms: NOW,
    registered: 900,
    ranked: 400,
    leader: 10_000,
    rows: [
      { rank: 1, name: "leader", score: 10_000, hands: 500, won: 100, rebuys: 0, ours: false },
      { rank: 3, name: "SurSvan", score: 5_000, hands: 400, won: 90, rebuys: 0, ours: true },
    ],
  };
  globalThis.location.pathname = "/";
  listeners.fleet({ data: JSON.stringify(next) });
  expect(ids.view.textContent).toContain("Season 17");
  const [roster] = byClass(ids.view, "management-roster");
  expect(roster.all("tbody")[0].all("tr")[0].textContent).toContain("#3");
  next.season.error = "timeout";
  listeners.fleet({ data: JSON.stringify(next) });
  expect(byClass(ids.view, "management-metrics")[0].textContent).toContain("previous read");
  globalThis.location.pathname = "/bot/SurSvan";
  listeners.fleet({ data: JSON.stringify(next) });
  const [tiles] = byClass(ids.view, "summary");
  expect(tiles.textContent).toContain("#3");
  expect(ids.view.textContent).toContain("5,000 behind leader");
});

test("the activity log tells each hand a bot counts while the page is open", () => {
  show("/");
  const next = fleet();
  next.bots[0].play = { ...next.bots[0].play, counted: 201, recent: [1_000, -3_000, 500, 2_000] };
  listeners.fleet({ data: JSON.stringify(next) });
  const [log] = byClass(ids.view, "log");
  expect(log.all("li")[0].textContent).toContain("SurSvanwon 2.0 bb");
});

test("a bot's table, and a chart with its numbers in a table", () => {
  const view = show("/bot/SurSvan");
  expect(view.textContent).toContain("Ah Kd 7c 2s");
  expect(view.textContent).toContain("SurSvan (us)");
  const [svg] = byClass(view, "chart");
  expect(svg.attrs.role).toBe("img");
  expect(svg.attrs["aria-label"]).toContain("-1.5 big blinds");
  expect(svg.querySelector("polyline").attrs.points.split(" ").length).toBe(4);
  // The fallback: one row a hand, under the chart.
  const rows = view.querySelector("details").all("tr");
  expect(rows.length).toBe(1 + 3);
  expect(rows[2].textContent).toBe("2-3.00-2.00");
});

test("an opponent's name is text, whatever it holds", () => {
  const view = show("/bot/SurSvan");
  expect(view.all("img").length).toBe(0);
  expect(view.innerHTML).toContain("&lt;img src=x onerror=alert(1)>");
});

test("the release page says where the slots point and what each bot runs", () => {
  const view = show("/release");
  expect(view.textContent).toContain("Current11.0.0");
  expect(view.textContent).toContain("Previous10.9.0");
  expect(view.textContent).toContain("10.9.0, 11.0.0");
  expect(view.all("tr").length).toBe(1 + 2);
  expect(view.textContent).toContain("308e7da4");
});

test("the round-robin page is one of the page's own views", () => {
  expect(show("/round-robin").textContent).toContain("No round-robin pass is recorded yet.");
});

test("a bot that has written no status says so", () => {
  expect(show("/bot/Nobody").textContent).toContain("This bot has written no status.");
});

test("the scoreboard has a whisker an opponent over the latest run, its numbers as text beside it", () => {
  const view = show("/scoreboard");
  expect(view.all("section").map((panel) => panel.querySelector("h2").textContent)).toEqual([
    "Latest run",
    "Trend",
  ]);
  expect(view.textContent).toContain("abc1234");
  // The champion is what the numbers mean: a promotion moves play under the same commit.
  expect(view.textContent).toContain("champion 308e7da4");
  expect(view.textContent).toContain("seed 7");
  // The Opponents table is ranked by win rate: a head row, then the best win rate first, whatever order
  // the run played them in. Here the fixture lists nit (+12,345.6) before maniac (-5,000).
  const opponents = view.all("table")[0];
  const rows = opponents.all("tr");
  expect(rows.length).toBe(1 + 2);
  expect(rows[1].textContent).toContain("+12345.6 mbb/100");
  expect(rows[2].textContent).toContain("−5000.0 mbb/100");
  expect(rows[1].all("svg").length).toBe(1);
  // The trend keeps each run, newest first, and a run that did not play an opponent is a dash.
  const trendRows = view.all("table")[1].all("tr");
  expect(trendRows.length).toBe(1 + 2);
  expect(trendRows[1].textContent).toContain("abc1234");
  expect(trendRows[2].textContent).toContain("older99");
  expect(trendRows[2].textContent).toContain("—");
});

test("the trend keeps an opponent the latest run did not play", () => {
  // The trend is across commits, so its columns are every opponent any run played. Taking them
  // from the newest run alone would drop an opponent the newest run skipped from every row.
  globalThis.location.pathname = "/scoreboard";
  const dropped = fleet();
  dropped.arena[0].rows = dropped.arena[0].rows.filter((row) => row.opponent === "nit");
  dropped.arena[1].rows = [
    { opponent: "nit", deals: 400, mbb_per_100: 1_000, ci95: 500 },
    { opponent: "maniac", deals: 400, mbb_per_100: 2_000, ci95: 500 },
  ];
  listeners.fleet({ data: JSON.stringify(dropped) });
  const trend = ids.view.all("table")[1];
  const head = trend.all("tr")[0].textContent;
  expect(head).toContain("maniac");
  // And the older run's line for it is kept, not dashed out.
  expect(trend.all("tr")[2].textContent).toContain("+2000.0 mbb/100");
});

test("two runs of one commit under different champions are told apart", () => {
  // A promotion changes play under one commit, so the commit alone does not name a run: the
  // trend would show two identical labels for two different bodies of play.
  globalThis.location.pathname = "/scoreboard";
  const promoted = fleet();
  promoted.arena[0].champion = "aaaa1111bbbb2222";
  promoted.arena[0].commit = promoted.arena[1].commit;
  listeners.fleet({ data: JSON.stringify(promoted) });
  const labels = ids.view
    .all("table")[1]
    .all("tr")
    .slice(1)
    .map((row) => row.all("td")[0].textContent);
  expect(new Set(labels).size).toBe(2);
});

test("the scoreboard displays legacy runs alongside runs with a champion", () => {
  globalThis.location.pathname = "/scoreboard";
  const mixed = fleet();
  mixed.arena[1].champion = null;
  listeners.fleet({ data: JSON.stringify(mixed) });
  expect(ids.view.textContent).toContain("older99 · unknown");
  expect(ids.view.textContent).toContain("champion 308e7da4");

  mixed.arena.shift();
  listeners.fleet({ data: JSON.stringify(mixed) });
  expect(ids.view.textContent).toContain("champion unknown");
});

test("the scoreboard is ranked by win rate, best first", () => {
  // The spec asks for a win-rate order, not the order the run played them in. The fixture's
  // lines are nit then maniac; reversed here so the ranking cannot happen by accident.
  globalThis.location.pathname = "/scoreboard";
  const reversed = fleet();
  reversed.arena[0].rows.reverse();
  listeners.fleet({ data: JSON.stringify(reversed) });
  const rows = ids.view.all("table")[0].all("tr");
  expect(rows[1].textContent).toContain("+12345.6 mbb/100");
  expect(rows[2].textContent).toContain("−5000.0 mbb/100");
});

test("a 6-max run is its own board beside heads up, never in the heads-up trend", () => {
  globalThis.location.pathname = "/scoreboard";
  const both = fleet();
  both.arena.unshift({ ...both.arena[0], commit: "sixmax1", league: "6-max" });
  listeners.fleet({ data: JSON.stringify(both) });
  const headings = ids.view.all("h2").map((heading) => heading.textContent);
  expect(headings.filter((text) => text.startsWith("Heads up ")).length).toBe(1);
  expect(headings.filter((text) => text.startsWith("6-max ")).length).toBe(1);
  // Heads up first, whichever league played last: its latest run is still abc1234.
  expect(headings.findIndex((t) => t.startsWith("Heads up"))).toBeLessThan(
    headings.findIndex((t) => t.startsWith("6-max")),
  );
  const tables = ids.view.all("table");
  expect(tables.length).toBe(4);
  // Heads up: latest run, then a trend of its own two runs only.
  expect(tables[1].all("tr").length).toBe(1 + 2);
  expect(tables[1].textContent).not.toContain("sixmax1");
  expect(tables[3].all("tr").length).toBe(1 + 1);
  expect(tables[3].textContent).toContain("sixmax1");
});

test("an opponent's name on the scoreboard is text too", () => {
  const view = show("/scoreboard");
  expect(view.all("b").length).toBe(0);
  expect(view.innerHTML).toContain("&lt;b>maniac&lt;/b>");
});

test("with no Arena run yet the scoreboard says how to make one", () => {
  globalThis.location.pathname = "/scoreboard";
  listeners.fleet({ data: JSON.stringify({ ...fleet(), arena: [] }) });
  expect(ids.view.textContent).toContain("svanbot arena");
});

test("a dropped stream keeps the last fleet on screen and says it dropped", () => {
  show("/");
  listeners.error();
  expect(ids.worst.textContent).toContain("The event stream dropped");
  expect(ids.view.all("article").length).toBe(2);
});

test("an open table fallback stays open when the page redraws", () => {
  const view = show("/bot/SurSvan");
  view.querySelector("details").open = true;
  const next = fleet();
  next.bots[0].hands += 1;
  listeners.fleet({ data: JSON.stringify(next) });
  expect(view.querySelector("details").open).toBe(true);
  expect(view.textContent).toContain("211");
});

test("a page inside the dashboard is reached without a reload: the path moves and the view follows", async () => {
  const { navigate } = await import("./app.js");
  show("/");
  navigate("/bot/SurSvan");
  expect(globalThis.location.pathname).toBe("/bot/SurSvan");
  expect(titles(ids.view)).toContain("Table");
  expect(ids.bots.all("a").map((tab) => tab.attrs["aria-current"] ?? null)).toEqual(["page", null]);
  navigate("/");
  expect(globalThis.location.pathname).toBe("/");
  expect(ids.view.all("article").length).toBe(2);
});
