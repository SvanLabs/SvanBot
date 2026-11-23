// The control room's panels (V11-44): the season race and a bot's standing, opponent intel, the
// highlights, the activity log and the notes. Every name on them is text, never markup.
import { beforeAll, expect, test } from "bun:test";
import { byClass, Node, standIn } from "./testdom.js";

let widgets;

beforeAll(async () => {
  globalThis.document = standIn;
  widgets = await import("./widgets.js");
});

/** Reads a panel's body: one node or a list of them. */
function body(...nodes) {
  const holder = new Node("div");
  holder.append(...nodes.flat(Number.POSITIVE_INFINITY).filter(Boolean));
  return holder;
}

const READ_MS = 1_800_000_000_000;

function season() {
  return {
    number: 17,
    ends: "2026-11-29T11:48:54+00:00",
    remaining_s: 3 * 86_400 + 5 * 3_600,
    registered: 1196,
    ranked: 412,
    leader: 1_000_000,
    read_ms: READ_MS,
    rows: [
      {
        rank: 1,
        name: "Synapse",
        score: 1_000_000,
        hands: 13_000,
        won: 4_000,
        rebuys: 0,
        ours: false,
      },
      { rank: 2, name: "SurSvan", score: 750_000, hands: 9_000, won: 3_000, rebuys: 0, ours: true },
      {
        rank: 3,
        name: "<b>rival</b>",
        score: 500_000,
        hands: 8_000,
        won: 2_000,
        rebuys: 1,
        ours: false,
      },
      { rank: 57, name: "Svanar", score: 20_000, hands: 4_000, won: 900, rebuys: 2, ours: true },
    ],
  };
}

test("the season race says the season and its time left, and marks our bots and the gap below the top", () => {
  const race = body(widgets.seasonRace(season(), READ_MS + 3_600_000));
  expect(race.textContent).toContain("Season 17");
  expect(race.textContent).toContain("3d 4h left");
  expect(race.textContent).toContain("412 ranked of 1,196");
  const rows = byClass(race, "season-row");
  expect(rows.map((row) => byClass(row, "season-name")[0].textContent)).toEqual([
    "Synapse",
    "SurSvan",
    "<b>rival</b>",
    "Svanar",
  ]);
  expect(rows.map((row) => (row.attrs.class.includes("ours") ? "ours" : ""))).toEqual([
    "",
    "ours",
    "",
    "ours",
  ]);
  // A bar is the score's share of the leader's.
  expect(byClass(rows[1], "season-bar")[0].attrs.style).toBe("--share:75.0%");
  // Rank 57 is not next to rank 3: the gap is drawn.
  expect(byClass(race, "season-gap").length).toBe(1);
  expect(race.all("b").length).toBe(0);
});

test("before the first read, and after a failed one, the race says so", () => {
  expect(body(widgets.seasonRace(null, READ_MS)).textContent).toContain("Reading the season");
  expect(
    body(widgets.seasonRace({ error: "/season/current: 503" }, READ_MS)).textContent,
  ).toContain("could not be read: /season/current: 503");
  const stale = body(widgets.seasonRace({ ...season(), error: "timed out" }, READ_MS));
  expect(stale.textContent).toContain("The last read failed (timed out)");
  expect(byClass(stale, "season-row").length).toBe(4);
});

test("a bot's standing is its rank, its score and what it trails by", () => {
  const sur = body(widgets.standing(season(), "SurSvan"));
  expect(sur.textContent).toContain("#2");
  expect(sur.textContent).toContain("of 412 ranked");
  expect(sur.textContent).toContain("750,000");
  expect(sur.textContent).toContain("250,000 behind Synapse");
  const leader = body(widgets.standing(season(), "Synapse"));
  expect(leader.textContent).toContain("leads the season");
  expect(body(widgets.standing(season(), "Svanism")).textContent).toContain(
    "not on the season board",
  );
});

const OPPONENTS = [
  { name: "zed", class: "fixed", change: null, hands: 80, actions: 120, earned_pct: 40 },
  { name: "<img src=x>", class: "switching", change: 512, hands: 30, actions: 12, earned_pct: 5 },
  { name: "amy", class: "unknown", change: null, hands: 3, actions: 0, earned_pct: 0 },
];

test("opponent intel counts the classes, reads the table first, then everyone by name", () => {
  const status = {
    bot: "SurSvan",
    reads_in_play: false,
    opponents: OPPONENTS,
    table: {
      seat: 1,
      players: [
        { seat: 0, name: "zed" },
        { seat: 1, name: "SurSvan" },
        { seat: 2, name: "stranger" },
      ],
    },
  };
  const intel = body(widgets.intel(status));
  const mix = byClass(intel, "mix")[0];
  expect(mix.textContent).toContain("1fixed");
  expect(mix.textContent).toContain("1switching");
  expect(mix.textContent).toContain("1unknown");
  const [here] = byClass(intel, "at-table");
  expect(here.textContent).toContain("zed");
  expect(here.textContent).toContain("stranger");
  expect(here.textContent).toContain("no read");
  expect(here.textContent).not.toContain("SurSvan");
  // Everyone, by name, never ranked.
  const names = byClass(intel, "intel-name").map((cell) => cell.textContent);
  expect(names.slice(-3)).toEqual(["<img src=x>", "amy", "zed"]);
  expect(intel.textContent).toContain("switching since hand 512");
  expect(intel.textContent).toContain("held");
  expect(intel.all("img").length).toBe(0);
});

test("the highlights are tiles of the best and worst hand, the streak and the form", () => {
  const tiles = body(widgets.highlightsView({ recent: [2_000, -6_500, 500, 1_250, 3_000] }));
  expect(tiles.textContent).toContain("+3.0 bb");
  expect(tiles.textContent).toContain("−6.5 bb");
  expect(tiles.textContent).toContain("3 won");
  expect(tiles.textContent).toContain("+5.0 bb/100");
  expect(body(widgets.highlightsView({ recent: [] })).textContent).toContain("No counted hands");
});

test("the activity log is newest first, a bot's own log only its own", () => {
  const log = [
    { bot: "Svanar", at: READ_MS + 2_000, tone: "down", text: "lost 1.0 bb" },
    { bot: "SurSvan", at: READ_MS + 1_000, tone: "up", text: "won 2.0 bb" },
  ];
  const fleet = body(widgets.activityView(log, null));
  expect(fleet.all("li").map((item) => item.textContent)).toEqual([
    `${widgets.clock(READ_MS + 2_000)}Svanarlost 1.0 bb`,
    `${widgets.clock(READ_MS + 1_000)}SurSvanwon 2.0 bb`,
  ]);
  const own = body(widgets.activityView(log, "SurSvan"));
  expect(own.all("li").map((item) => item.textContent)).toEqual([
    `${widgets.clock(READ_MS + 1_000)}won 2.0 bb`,
  ]);
  expect(body(widgets.activityView([], null)).textContent).toContain("Nothing has happened");
});

test("a note is the reader's own: kept as typed, saved under its key", () => {
  const note = widgets.notes("SurSvan", "watch the river overbets");
  const [area] = note.all("textarea");
  expect(area.attrs["data-keep"]).toBe("SurSvan");
  expect(area.attrs["data-note"]).toBe("SurSvan");
  expect(area.textContent).toBe("watch the river overbets");
});
