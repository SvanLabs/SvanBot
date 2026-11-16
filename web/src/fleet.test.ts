import { describe, expect, test } from "bun:test";

import {
  type BotStatus,
  botView,
  duration,
  type Fleet,
  fleetProblems,
  freshness,
  parseFleet,
} from "./fleet";

function bot(overrides: Partial<BotStatus> = {}): BotStatus {
  return {
    bot: "SurSvan",
    pid: 42,
    version: "11.0.0-dev",
    started_ms: 1_000,
    updated_ms: 10_000,
    connected: true,
    table: null,
    hands: 12,
    incidents: 0,
    last_hand_ms: 9_000,
    last_error: null,
    ...overrides,
  };
}

function fleet(bots: BotStatus[] = [bot()], problems: string[] = []): Fleet {
  return { server_ms: 10_000, bots, problems };
}

describe("parseFleet", () => {
  test("accepts the document the server sends", () => {
    const parsed = parseFleet(fleet());
    expect(parsed?.bots[0]?.bot).toBe("SurSvan");
  });

  test("refuses a document that is not a fleet", () => {
    expect(parseFleet(null)).toBeNull();
    expect(parseFleet({})).toBeNull();
    expect(parseFleet({ server_ms: 1, bots: "no", problems: [] })).toBeNull();
    expect(parseFleet({ server_ms: 1, bots: [{ hands: 1 }], problems: [] })).toBeNull();
  });
});

describe("botView", () => {
  test("a connected bot at a table is playing", () => {
    const view = botView(
      bot({
        table: {
          id: "t-1",
          seat: 2,
          hand_id: "h-1",
          street: "flop",
          board: ["Ah", "Kd", "7c"],
          pot: 300,
          big_blind: 40,
          dealer: 0,
          players: [],
        },
      }),
      12_000,
    );
    expect(view.tone).toBe("ok");
    expect(view.label).toBe("Playing");
    expect(view.detail).toContain("t-1");
    expect(view.detail).toContain("flop");
  });

  test("a connected bot with no table is waiting", () => {
    const view = botView(bot(), 12_000);
    expect(view.tone).toBe("warn");
    expect(view.label).toBe("Waiting");
  });

  test("a disconnected bot is offline and says why", () => {
    const view = botView(
      bot({ connected: false, last_error: "the venue refused the key" }),
      12_000,
    );
    expect(view.tone).toBe("bad");
    expect(view.label).toBe("Offline");
    expect(view.detail).toBe("the venue refused the key");
  });

  test("a bot that stopped reporting is stale, even if its last status said connected", () => {
    const view = botView(bot({ updated_ms: 1_000 }), 30_000);
    expect(view.tone).toBe("bad");
    expect(view.label).toBe("No recent status");
    expect(view.detail).toContain("29s");
  });
});

describe("fleetProblems", () => {
  test("an all-clear fleet has nothing to report", () => {
    expect(fleetProblems(fleet(), 12_000, null)).toEqual([]);
  });

  test("a stale bot, an offline bot, an incident and a broken status are all reported", () => {
    const problems = fleetProblems(
      fleet([
        bot({ bot: "Alpha", updated_ms: 1_000 }),
        bot({ bot: "Beta", connected: false, last_error: "no key" }),
        bot({ bot: "Gamma", incidents: 2 }),
      ]),
      30_000,
      "the stream dropped",
    );
    const text = problems.map((problem) => problem.text).join("\n");
    expect(text).toContain("Alpha has not reported");
    expect(text).toContain("Beta is offline: no key");
    expect(text).toContain("Gamma raised 2 fair-play incidents");
    expect(text).toContain("the stream dropped");
    expect(problems.filter((problem) => problem.tone === "bad")).toHaveLength(3);
  });

  test("an empty fleet says so", () => {
    const problems = fleetProblems(fleet([]), 12_000, null);
    expect(problems[0]?.text).toContain("No bot has reported");
  });
});

describe("freshness", () => {
  test("before the first message it is connecting", () => {
    expect(freshness(null, 1_000).tone).toBe("warn");
  });

  test("a recent message is live, a late one is delayed, a lost one is bad", () => {
    expect(freshness(10_000, 12_000)).toEqual({ tone: "ok", label: "live" });
    expect(freshness(10_000, 20_000).tone).toBe("warn");
    expect(freshness(10_000, 60_000).tone).toBe("bad");
  });
});

describe("duration", () => {
  test("reads in seconds, minutes, hours and days", () => {
    expect(duration(500)).toBe("less than a second");
    expect(duration(29_000)).toBe("29s");
    expect(duration(120_000)).toBe("2m");
    expect(duration(3 * 3_600_000)).toBe("3h");
    expect(duration(2 * 86_400_000)).toBe("2d");
  });
});
