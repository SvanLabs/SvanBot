import { describe, expect, test } from "bun:test";

import { emptyView, receiveFleet, receiveTick, serverNow, streamFailed, type View } from "./view";

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
      run_incidents: 0,
      last_hand_ms: 9_000,
      last_error: null,
    },
  ],
  problems: [],
});

describe("receiveFleet", () => {
  test("records the fleet and both clocks", () => {
    const view = receiveFleet(emptyView, document, 20_000);
    expect(view.fleet?.bots[0]?.bot).toBe("SurSvan");
    expect(view.serverMs).toBe(10_000);
    expect(view.receivedAt).toBe(20_000);
    expect(view.error).toBeNull();
  });

  test("refuses a document it cannot parse and keeps what it had", () => {
    const good = receiveFleet(emptyView, document, 20_000);
    const view = receiveFleet(good, '{"bots":42}', 30_000);
    expect(view).toBe(good);
  });

  test("a good message clears an earlier stream error", () => {
    const broken = streamFailed(emptyView, "the stream dropped");
    const view = receiveFleet(broken, document, 20_000);
    expect(view.error).toBeNull();
  });

  test("a fleet older than the last message is ignored, so a slow fetch cannot undo an event", () => {
    const ticked = receiveTick(emptyView, '{"server_ms":20000}', 21_000);
    expect(receiveFleet(ticked, document, 22_000)).toBe(ticked);
  });
});

describe("receiveTick", () => {
  test("proves the stream alive without inventing a fleet", () => {
    const view = receiveTick(emptyView, '{"server_ms":10500}', 20_000);
    expect(view.fleet).toBeNull();
    expect(view.serverMs).toBe(10_500);
    expect(view.receivedAt).toBe(20_000);
  });

  test("a tick with no clock is ignored", () => {
    expect(receiveTick(emptyView, "{}", 20_000)).toBe(emptyView);
    expect(receiveTick(emptyView, "not json", 20_000)).toBe(emptyView);
  });
});

describe("streamFailed", () => {
  test("keeps the last fleet and says the stream is down", () => {
    const view = streamFailed(receiveFleet(emptyView, document, 20_000), "the stream dropped");
    expect(view.fleet?.bots).toHaveLength(1);
    expect(view.error).toBe("the stream dropped");
  });
});

describe("the server clock", () => {
  test("advances with local time, so a skewed browser clock does not distort ages", () => {
    const view: View = receiveFleet(emptyView, document, 20_000);
    expect(serverNow(view, 23_000)).toBe(13_000);
  });

  test("before the first message there is no clock", () => {
    expect(serverNow(emptyView, 23_000)).toBeNull();
  });
});
