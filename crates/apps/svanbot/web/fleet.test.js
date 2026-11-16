import { expect, test } from "bun:test";
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
  expect(route("/bot/SurSvan")).toEqual({ view: "bot", bot: "SurSvan" });
  expect(route("/bot/<script>")).toEqual({ view: "fleet" });
});

test("a document that is not a fleet is refused", () => {
  expect(isFleet({ server_ms: 1, bots: [bot()], problems: [] })).toBe(true);
  expect(isFleet({ server_ms: 1, bots: [{}], problems: [] })).toBe(false);
  expect(isFleet(null)).toBe(false);
});
