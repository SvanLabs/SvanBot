// The live feed the page renders: the last fleet, the dashboard's clock at the moment it arrived,
// and the local clock at that moment. Holding both clocks lets the page age a bot without trusting
// the browser's clock. Pure state transitions, so they are tested without a browser.

import { type Fleet, parseFleet } from "./fleet";

export interface Feed {
  /** The last fleet the dashboard sent, or `null` before the first one. */
  fleet: Fleet | null;
  /** The dashboard's clock when it sent the last fleet or tick. */
  serverMs: number | null;
  /** This browser's clock when that message arrived. */
  receivedAt: number | null;
  /** Why the stream is down, if it is. The last fleet stays on screen while it reconnects. */
  error: string | null;
}

export const emptyFeed: Feed = {
  fleet: null,
  serverMs: null,
  receivedAt: null,
  error: null,
};

/** Takes a `fleet` event. A document the page cannot trust changes nothing. */
export function receiveFleet(feed: Feed, data: string, localNow: number): Feed {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return feed;
  }
  const fleet = parseFleet(value);
  if (fleet === null) {
    return feed;
  }
  if (feed.serverMs !== null && fleet.server_ms < feed.serverMs) {
    return feed;
  }
  return {
    fleet,
    serverMs: fleet.server_ms,
    receivedAt: localNow,
    error: null,
  };
}

/** Takes a `tick` event: a fresh clock, and no change to the fleet. */
export function receiveTick(feed: Feed, data: string, localNow: number): Feed {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return feed;
  }
  if (
    typeof value !== "object" ||
    value === null ||
    typeof (value as { server_ms?: unknown }).server_ms !== "number"
  ) {
    return feed;
  }
  return {
    fleet: feed.fleet,
    serverMs: (value as { server_ms: number }).server_ms,
    receivedAt: localNow,
    error: null,
  };
}

/** Notes a lost stream. The last fleet stays visible, marked as the last known one. */
export function streamFailed(feed: Feed, error: string): Feed {
  if (feed.error === error) {
    return feed;
  }
  return { ...feed, error };
}

/** The dashboard's clock now, estimated from the last message, or `null` before one. */
export function serverNow(feed: Feed, localNow: number): number | null {
  if (feed.serverMs === null || feed.receivedAt === null) {
    return null;
  }
  return feed.serverMs + Math.max(0, localNow - feed.receivedAt);
}
