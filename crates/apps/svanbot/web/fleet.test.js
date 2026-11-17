import { expect, test } from "bun:test";
import {
  arenaRun,
  arenaSpan,
  botView,
  chart,
  duration,
  fleetProblems,
  freshness,
  isFleet,
  mbbText,
  rateText,
  route,
  running,
  trend,
  whisker,
  winRate,
} from "./fleet.js";

const NOW = 100_000;

function bot(overrides = {}) {
  return {
    bot: "SurSvan",
    updated_ms: NOW - 1_000,
    connected: true,
    table: {
      id: "t-1",
      street: "flop",
      pot: 240,
      board: ["Ah", "Kd", "7c"],
      players: [],
    },
    hands: 10,
    incidents: 0,
    run_incidents: 0,
    last_error: null,
    play: { champion: "308e7da4b29fd533", probe: null, counted: 0, won_mbb: 0, recent: [] },
    ...overrides,
  };
}

test("a bot in a hand is playing, and says where", () => {
  expect(botView(bot(), NOW)).toEqual({
    tone: "ok",
    label: "Playing",
    detail: "t-1 · flop · pot 240",
  });
});

test("a silent bot is unknown, whatever its last status said", () => {
  const view = botView(bot({ updated_ms: NOW - 16_000 }), NOW);
  expect(view.label).toBe("No recent status");
  expect(view.detail).toBe("no status for 16s");
});

test("an offline bot gives the reason, and a seated one waits", () => {
  expect(botView(bot({ connected: false, last_error: "socket: reset" }), NOW).detail).toBe(
    "socket: reset",
  );
  expect(botView(bot({ table: null }), NOW).label).toBe("Waiting");
});

test("the strip raises one line a bot, an incident before anything else of that bot's", () => {
  const fleet = {
    bots: [
      bot({ bot: "A", incidents: 5, run_incidents: 2, connected: false }),
      bot({ bot: "B", connected: false }),
      bot({ bot: "C", updated_ms: NOW - 60_000 }),
      bot({ bot: "D" }),
    ],
    problems: ["Broken.status.json: unreadable"],
  };
  expect(fleetProblems(fleet, NOW, "the stream dropped").map((problem) => problem.text)).toEqual([
    "the stream dropped",
    "A raised 2 fair-play incidents this run",
    "B is offline: the venue connection is closed",
    "C has not reported for 1m",
    "Broken.status.json: unreadable",
  ]);
  // An incident from a past run is on record, and raises nothing now.
  expect(fleetProblems({ bots: [bot({ incidents: 3 })], problems: [] }, NOW, null)).toEqual([]);
  expect(fleetProblems({ bots: [], problems: [] }, NOW, null)[0].text).toBe(
    "No bot has reported yet",
  );
});

test("freshness goes from live to delayed to lost", () => {
  expect(freshness(null, NOW).label).toBe("connecting");
  expect(freshness(NOW - 5_000, NOW).label).toBe("live");
  expect(freshness(NOW - 10_000, NOW).label).toBe("delayed");
  expect(freshness(NOW - 20_000, NOW).label).toBe("lost");
});

test("a duration reads in its coarsest unit", () => {
  expect([400, 59_000, 60_000, 3_600_000, 90_000_000].map(duration)).toEqual([
    "less than a second",
    "59s",
    "1m",
    "1h",
    "1d",
  ]);
});

test("the win rate is big blinds a hundred hands, from thousandths a hand", () => {
  // 3 big blinds over 200 hands is 1.5 bb/100.
  expect(winRate({ counted: 200, won_mbb: 3_000 })).toBe(1.5);
  expect(winRate({ counted: 0, won_mbb: 0 })).toBe(null);
  expect(rateText(1.5)).toBe("+1.5 bb/100");
  expect(rateText(-12.34)).toBe("−12.3 bb/100");
  expect(rateText(null)).toBe("no counted hands");
});

test("the chart runs from zero through the running total", () => {
  expect(running([1_000, -3_000, 500])).toEqual([1, -2, -1.5]);
  // Values 0, 1, -2 in a 100 by 30 box: the top is 1, the bottom -2, zero a third of the way down.
  expect(chart([1, -2], 100, 30)).toEqual({ points: "0,10 50,0 100,30", zero: 10 });
  expect(chart([], 100, 30).points).toBe("0,30");
});

test("a path names its view, and a name that is not a bot's falls back to the fleet", () => {
  expect(route("/")).toEqual({ view: "fleet" });
  expect(route("/release")).toEqual({ view: "release" });
  expect(route("/scoreboard")).toEqual({ view: "scoreboard" });
  expect(route("/bot/SurSvan")).toEqual({ view: "bot", bot: "SurSvan" });
  expect(route("/bot/<script>")).toEqual({ view: "fleet" });
});

test("a document that is not a fleet is refused", () => {
  expect(isFleet({ server_ms: 1, bots: [bot()], problems: [] })).toBe(true);
  expect(isFleet({ server_ms: 1, bots: [{}], problems: [] })).toBe(false);
  expect(isFleet(null)).toBe(false);
});

const RUN = {
  commit: "abc1234",
  seed: 7,
  rows: [
    { opponent: "nit", deals: 400, mbb_per_100: 12_345.6, ci95: 2_000.4 },
    { opponent: "maniac", deals: 400, mbb_per_100: -5_000, ci95: 9_000 },
  ],
};

test("the latest run is the first the fleet carries, and none is null", () => {
  expect(arenaRun([RUN, { ...RUN, commit: "older" }]).commit).toBe("abc1234");
  expect(arenaRun([])).toBe(null);
  expect(arenaRun(undefined)).toBe(null);
});

test("a win rate on the scoreboard is thousands of a big blind a hundred hands, signed", () => {
  // The Arena's number is already per hundred hands, in thousandths of a big blind.
  expect(mbbText(12_345.6)).toBe("+12345.6 mbb/100");
  expect(mbbText(-5_000)).toBe("−5000.0 mbb/100");
  expect(mbbText(0)).toBe("+0.0 mbb/100");
});

test("a whisker spans a run's interval in a box, clamped to it, with zero marked", () => {
  // 0 to 20,000 mbb over 100 by 10: 12,000 ± 2,000 is 10..14, so x from 50 to 70; zero at 0.
  const span = { low: 0, high: 20_000 };
  expect(whisker(12_000, 2_000, span, 100)).toEqual({ from: 50, to: 70, zero: 0, mean: 60 });
  // An interval running past the box is clamped to its edges, never drawn outside it.
  expect(whisker(19_000, 4_000, span, 100)).toEqual({ from: 75, to: 100, zero: 0, mean: 95 });
  // A span of nothing is the whole box, not a division by zero.
  expect(whisker(0, 0, { low: 0, high: 0 }, 100)).toEqual({ from: 0, to: 0, zero: 0, mean: 0 });
});

test("the trend is a run's mean against one opponent, oldest first", () => {
  const runs = [
    { ...RUN, commit: "new", seed: 1 },
    { ...RUN, commit: "mid", seed: 2 },
    { ...RUN, commit: "old", seed: 3 },
  ];
  expect(trend(runs, "nit")).toEqual([
    { commit: "old", mbb_per_100: 12_345.6 },
    { commit: "mid", mbb_per_100: 12_345.6 },
    { commit: "new", mbb_per_100: 12_345.6 },
  ]);
  // An opponent a run did not play is left out, not drawn as zero.
  expect(trend(runs, "absent")).toEqual([]);
});

test("the scoreboard's span reaches every interval it draws, and always zero", () => {
  // Nit runs to 14,346; the maniac's interval reaches -14,000 and +4,000.
  const span = arenaSpan(RUN.rows);
  expect(span).toEqual({ low: -14_000, high: 14_346 });
  // Zero is inside the span, so a zero line falls on the box, left of a run above it.
  const line = whisker(12_345.6, 2_000.4, span, 100);
  expect(line.zero).toBeGreaterThan(0);
  expect(line.zero).toBeLessThan(line.from);
  expect(line.from).toBeLessThan(line.mean);
  expect(line.mean).toBeLessThan(line.to);
  // A run whose every interval is positive still draws zero, at the box's edge.
  expect(arenaSpan([{ opponent: "x", deals: 1, mbb_per_100: 500, ci95: 100 }])).toEqual({
    low: 0,
    high: 600,
  });
});
