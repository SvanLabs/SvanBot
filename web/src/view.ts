// The live view the page renders: the last fleet, the dashboard's clock at the moment it arrived,
// and the local clock at that moment. Holding both clocks lets the page age a bot without trusting
// the browser's clock. Pure state transitions, so they are tested without a browser.

import { type Fleet, parseFleet } from "./fleet";

export interface View {
  /** The last fleet the dashboard sent, or `null` before the first one. */
  fleet: Fleet | null;
  /** The dashboard's clock when it sent the last fleet or tick. */
  serverMs: number | null;
  /** This browser's clock when that message arrived. */
  receivedAt: number | null;
  /** Why the stream is down, if it is. The last fleet stays on screen while it reconnects. */
  error: string | null;
}

export const emptyView: View = {
  fleet: null,
  serverMs: null,
  receivedAt: null,
  error: null,
};

/** Takes a `fleet` event. A document the page cannot trust changes nothing. */
export function receiveFleet(view: View, data: string, localNow: number): View {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return view;
  }
  const fleet = parseFleet(value);
  if (fleet === null) {
    return view;
  }
  if (view.serverMs !== null && fleet.server_ms < view.serverMs) {
    return view;
  }
  return {
    fleet,
    serverMs: fleet.server_ms,
    receivedAt: localNow,
    error: null,
  };
}

/** Takes a `tick` event: a fresh clock, and no change to the fleet. */
export function receiveTick(view: View, data: string, localNow: number): View {
  let value: unknown;
  try {
    value = JSON.parse(data);
  } catch {
    return view;
  }
  if (
    typeof value !== "object" ||
    value === null ||
    typeof (value as { server_ms?: unknown }).server_ms !== "number"
  ) {
    return view;
  }
  return {
    fleet: view.fleet,
    serverMs: (value as { server_ms: number }).server_ms,
    receivedAt: localNow,
    error: null,
  };
}

/** Notes a lost stream. The last fleet stays visible, marked as the last known one. */
export function streamFailed(view: View, error: string): View {
  if (view.error === error) {
    return view;
  }
  return { ...view, error };
}

/** The dashboard's clock now, estimated from the last message, or `null` before one. */
export function serverNow(view: View, localNow: number): number | null {
  if (view.serverMs === null || view.receivedAt === null) {
    return null;
  }
  return view.serverMs + Math.max(0, localNow - view.receivedAt);
}
