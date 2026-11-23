// The activity log and the highlights (V11-44): what changed between two fleets, as the operator
// would want it told, and a bot's best and worst hands, streak and form.
import { expect, test } from "bun:test";
import { changes, highlights } from "./activity.js";

function bot(name, over = {}) {
  return {
    bot: name,
    connected: true,
    version: "11.1.3",
    incidents: 0,
    last_error: null,
    table: { id: "table-aaaa1111" },
    play: { champion: "308e7da4b29fd533", counted: 10, recent: [1_000, -2_000] },
    ...over,
  };
}

function fleet(bots, season = null) {
  return { server_ms: 5_000, bots, season };
}

test("the first fleet says nothing: there is nothing yet to compare it with", () => {
  expect(changes(null, fleet([bot("SurSvan")]))).toEqual([]);
});

test("each change of a bot is an event of that bot, told plainly and toned", () => {
  const before = fleet([bot("SurSvan"), bot("Svanar"), bot("Svanism")]);
  const after = fleet([
    bot("SurSvan", {
      table: { id: "table-bbbb2222" },
      play: { champion: "308e7da4b29fd533", counted: 12, recent: [1_000, -2_000, 3_500, -250] },
    }),
    bot("Svanar", { connected: false, last_error: "socket: reset", table: null }),
    bot("Svanism", {
      incidents: 1,
      version: "11.1.4",
      play: { champion: "aaaabbbbccccdddd", counted: 10, recent: [1_000, -2_000] },
    }),
  ]);
  const told = changes(before, after).map((event) => [event.bot, event.tone, event.text]);
  expect(told).toEqual([
    ["SurSvan", "", "sat at table bbbb2222"],
    ["SurSvan", "up", "won 3.5 bb"],
    ["SurSvan", "down", "lost 0.3 bb"],
    ["Svanar", "bad", "went offline: socket: reset"],
    ["Svanism", "bad", "recorded an incident"],
    ["Svanism", "", "now runs build 11.1.4"],
    ["Svanism", "", "plays champion aaaabbbb"],
  ]);
  expect(changes(before, after).every((event) => event.at === 5_000)).toBe(true);
});

test("a bot back online, a bot new to the fleet and a move on the season board are events", () => {
  const season = (rank) => ({ rows: [{ name: "SurSvan", rank, ours: true }] });
  const before = fleet([bot("SurSvan", { connected: false })], season(12));
  const after = fleet([bot("SurSvan"), bot("Svanism")], season(9));
  const told = changes(before, after).map((event) => [event.bot, event.tone, event.text]);
  expect(told).toEqual([
    ["SurSvan", "ok", "came online"],
    ["Svanism", "ok", "joined the fleet"],
    ["SurSvan", "up", "climbed to #9 on the season board"],
  ]);
  const fell = changes(after, fleet(after.bots, season(15)));
  expect(fell.map((event) => event.text)).toEqual(["fell to #15 on the season board"]);
});

test("the highlights are a bot's best and worst hand in big blinds, its streak, and its form in bb/100", () => {
  const seen = highlights({ recent: [2_000, -6_500, 500, 1_250, 3_000] });
  expect(seen).toEqual({
    hands: 5,
    best: 3,
    worst: -6.5,
    streak: { won: true, length: 3 },
    form: 5,
  });
  expect(highlights({ recent: [] })).toBe(null);
});
