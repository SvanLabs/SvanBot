import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";

import { BotCard, EmptyFleet, FleetView, StatusStrip, type StripFreshness } from "./components";
import { emptyFeed, receiveFleet } from "./feed";
import type { BotStatus, Problem, TableStatus } from "./fleet";

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

function table(overrides: Partial<TableStatus> = {}): TableStatus {
  return {
    id: "t-1",
    seat: 2,
    hand_id: "h-1",
    street: "flop",
    board: ["Ah", "Kd", "7c"],
    pot: 300,
    big_blind: 40,
    dealer: 0,
    players: [
      { seat: 0, name: "Rival", stack: 1_200, folded: false },
      { seat: 2, name: "SurSvan", stack: 980, folded: false },
    ],
    ...overrides,
  };
}

const fresh: StripFreshness = { tone: "ok", label: "live" };
const problem: Problem = { tone: "bad", text: "Beta is offline: no key" };

describe("BotCard", () => {
  test("a playing bot names the table, street, board and players", () => {
    const html = renderToStaticMarkup(<BotCard status={bot({ table: table() })} nowMs={12_000} />);
    expect(html).toContain('data-bot="SurSvan"');
    expect(html).toContain("Playing");
    expect(html).toContain("t-1");
    expect(html).toContain("flop");
    expect(html).toContain("pot 300");
    expect(html).toContain("Ah Kd 7c");
    expect(html).toContain("Rival");
    expect(html).toContain("1,200");
    expect(html).toContain("12 hands");
  });

  test("the board keeps the venue's order", () => {
    const html = renderToStaticMarkup(
      <BotCard status={bot({ table: table({ board: ["7c", "Ah", "Kd"] }) })} nowMs={12_000} />,
    );
    expect(html).toContain("7c Ah Kd");
  });

  test("our seat is marked, and a folded rival is too", () => {
    const html = renderToStaticMarkup(
      <BotCard
        status={bot({
          table: table({
            players: [
              { seat: 0, name: "Rival", stack: 1_200, folded: true },
              { seat: 2, name: "SurSvan", stack: 980, folded: false },
            ],
          }),
        })}
        nowMs={12_000}
      />,
    );
    expect(html).toContain("you");
    expect(html).toContain("folded");
  });

  test("a waiting bot says it is connected with no table", () => {
    const html = renderToStaticMarkup(<BotCard status={bot()} nowMs={12_000} />);
    expect(html).toContain("Waiting");
    expect(html).toContain("connected, no table");
  });

  test("an offline bot says why, and does not claim a table", () => {
    const html = renderToStaticMarkup(
      <BotCard
        status={bot({ connected: false, last_error: "the venue refused the key" })}
        nowMs={12_000}
      />,
    );
    expect(html).toContain("Offline");
    expect(html).toContain("the venue refused the key");
    expect(html).not.toContain("pot");
  });

  test("a silent bot is stale, not playing", () => {
    const html = renderToStaticMarkup(
      <BotCard status={bot({ table: table(), updated_ms: 1_000 })} nowMs={30_000} />,
    );
    expect(html).toContain("No recent status");
    expect(html).toContain("29s");
    expect(html).not.toContain("Playing");
  });

  test("a bot name is escaped, never markup", () => {
    const html = renderToStaticMarkup(
      <BotCard status={bot({ bot: "<script>alert(1)</script>" })} nowMs={12_000} />,
    );
    expect(html).not.toContain("<script>alert(1)</script>");
    expect(html).toContain("&lt;script&gt;");
  });
});

describe("StatusStrip", () => {
  test("summarises the fleet and shows the freshest state", () => {
    const html = renderToStaticMarkup(
      <StatusStrip
        bots={[
          bot({ table: table() }),
          bot({ bot: "Beta", table: null }),
          bot({ bot: "Gamma", connected: false }),
        ]}
        problems={[]}
        fresh={fresh}
        nowMs={12_000}
      />,
    );
    expect(html).toContain("3 bots");
    expect(html).toContain("1 playing");
    expect(html).toContain("1 waiting");
    expect(html).toContain("1 offline");
    expect(html).toContain("live");
    expect(html).toContain("All quiet");
    expect(html).toContain('class="strip"');
  });

  test("puts the worst problem in the strip and counts the rest", () => {
    const html = renderToStaticMarkup(
      <StatusStrip
        bots={[bot()]}
        problems={[problem, { tone: "warn", text: "later" }]}
        fresh={fresh}
        nowMs={12_000}
      />,
    );
    expect(html).toContain("Beta is offline: no key");
    expect(html).toContain("+1 more");
  });

  test("a silent bot is counted as not reporting, not as playing", () => {
    const html = renderToStaticMarkup(
      <StatusStrip
        bots={[bot({ table: table(), updated_ms: 1_000 })]}
        problems={[]}
        fresh={fresh}
        nowMs={30_000}
      />,
    );
    expect(html).toContain("1 not reporting");
    expect(html).toContain("0 playing");
    expect(html).not.toContain("1 playing");
  });

  test("marks the strip with the freshness tone", () => {
    const html = renderToStaticMarkup(
      <StatusStrip
        bots={[bot()]}
        problems={[]}
        fresh={{ tone: "bad", label: "lost" }}
        nowMs={12_000}
      />,
    );
    expect(html).toContain("tone-bad");
    expect(html).toContain("lost");
  });
});

describe("EmptyFleet", () => {
  test("says no bot has reported and how to fix it", () => {
    const html = renderToStaticMarkup(<EmptyFleet />);
    expect(html).toContain("No bot has reported yet");
    expect(html).toContain("SVANBOT_DATA");
  });
});

describe("FleetView", () => {
  test("before the first message it is connecting and empty", () => {
    const html = renderToStaticMarkup(<FleetView feed={emptyFeed} localNow={5_000} />);
    expect(html).toContain("connecting");
    expect(html).toContain("No bot has reported yet");
  });

  test("after a fleet message it shows one card per bot", () => {
    const feed = receiveFleet(
      emptyFeed,
      JSON.stringify({
        server_ms: 10_000,
        bots: [bot(), bot({ bot: "Beta" })],
        problems: [],
      }),
      11_000,
    );
    const html = renderToStaticMarkup(<FleetView feed={feed} localNow={12_000} />);
    expect(html).toContain("2 bots");
    expect(html).toContain('data-bot="SurSvan"');
    expect(html).toContain('data-bot="Beta"');
    expect(html).toContain("live");
  });

  test("a dropped stream keeps the cards and marks the feed down", () => {
    const feed = {
      ...receiveFleet(
        emptyFeed,
        JSON.stringify({ server_ms: 10_000, bots: [bot()], problems: [] }),
        11_000,
      ),
      error: "the event stream dropped; retrying",
    };
    const html = renderToStaticMarkup(<FleetView feed={feed} localNow={40_000} />);
    expect(html).toContain('data-bot="SurSvan"');
    expect(html).toContain("the event stream dropped");
    expect(html).toContain("lost");
  });

  test("every problem after the worst one is still on the page", () => {
    const feed = receiveFleet(
      emptyFeed,
      JSON.stringify({
        server_ms: 10_000,
        bots: [
          bot({ bot: "Alpha", connected: false, last_error: "no key" }),
          bot({ bot: "Beta", connected: false, last_error: "a bad seat" }),
        ],
        problems: ["Broken.status.json: not a status"],
      }),
      11_000,
    );
    const html = renderToStaticMarkup(<FleetView feed={feed} localNow={12_000} />);
    expect(html).toContain("Alpha is offline: no key");
    expect(html).toContain("Beta is offline: a bad seat");
    expect(html).toContain("Broken.status.json: not a status");
  });
});
