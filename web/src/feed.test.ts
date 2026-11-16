import { describe, expect, test } from "bun:test";

import { emptyFeed, type Feed, receiveFleet, receiveTick, serverNow, streamFailed } from "./feed";

const document = JSON.stringify({
  server_ms: 10_000,
  bots: [
    {
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
    },
  ],
  problems: [],
});

describe("receiveFleet", () => {
  test("records the fleet and both clocks", () => {
    const feed = receiveFleet(emptyFeed, document, 20_000);
    expect(feed.fleet?.bots[0]?.bot).toBe("SurSvan");
    expect(feed.serverMs).toBe(10_000);
    expect(feed.receivedAt).toBe(20_000);
    expect(feed.error).toBeNull();
  });

  test("refuses a document it cannot parse and keeps what it had", () => {
    const good = receiveFleet(emptyFeed, document, 20_000);
    const feed = receiveFleet(good, '{"bots":42}', 30_000);
    expect(feed).toBe(good);
  });

  test("a good message clears an earlier stream error", () => {
    const broken = streamFailed(emptyFeed, "the stream dropped");
    const feed = receiveFleet(broken, document, 20_000);
    expect(feed.error).toBeNull();
  });

  test("a fleet older than the last message is ignored, so a slow fetch cannot undo an event", () => {
    const ticked = receiveTick(emptyFeed, '{"server_ms":20000}', 21_000);
    expect(receiveFleet(ticked, document, 22_000)).toBe(ticked);
  });
});

describe("receiveTick", () => {
  test("proves the stream alive without inventing a fleet", () => {
    const feed = receiveTick(emptyFeed, '{"server_ms":10500}', 20_000);
    expect(feed.fleet).toBeNull();
    expect(feed.serverMs).toBe(10_500);
    expect(feed.receivedAt).toBe(20_000);
  });

  test("a tick with no clock is ignored", () => {
    expect(receiveTick(emptyFeed, "{}", 20_000)).toBe(emptyFeed);
    expect(receiveTick(emptyFeed, "not json", 20_000)).toBe(emptyFeed);
  });
});

describe("streamFailed", () => {
  test("keeps the last fleet and says the stream is down", () => {
    const feed = streamFailed(receiveFleet(emptyFeed, document, 20_000), "the stream dropped");
    expect(feed.fleet?.bots).toHaveLength(1);
    expect(feed.error).toBe("the stream dropped");
  });
});

describe("the server clock", () => {
  test("advances with local time, so a skewed browser clock does not distort ages", () => {
    const feed: Feed = receiveFleet(emptyFeed, document, 20_000);
    expect(serverNow(feed, 23_000)).toBe(13_000);
  });

  test("before the first message there is no clock", () => {
    expect(serverNow(emptyFeed, 23_000)).toBeNull();
  });
});
