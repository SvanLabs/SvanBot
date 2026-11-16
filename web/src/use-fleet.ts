"use client";

// The page's one live connection (V11-11): an initial fleet fetch, then the dashboard's event
// stream, with a local clock ticking once a second so ages advance between messages.

import { useEffect, useState } from "react";

import { emptyFeed, type Feed, receiveFleet, receiveTick, streamFailed } from "./feed";

const CLOCK_MS = 1_000;
const DROPPED = "the event stream dropped; retrying";

export interface Live {
  feed: Feed;
  /** This browser's clock, updated once a second. */
  localNow: number;
}

export function useFleet(): Live {
  const [feed, setFeed] = useState<Feed>(emptyFeed);
  const [localNow, setLocalNow] = useState(0);

  useEffect(() => {
    setLocalNow(Date.now());
    const timer = setInterval(() => setLocalNow(Date.now()), CLOCK_MS);
    return () => clearInterval(timer);
  }, []);

  // The fetch is the fallback for a browser whose stream never opens; a failure here is not
  // worth showing, because the stream's own error is the useful one.
  useEffect(() => {
    let live = true;
    fetch("/api/fleet", { cache: "no-store" })
      .then((response) => (response.ok ? response.text() : Promise.reject(response.status)))
      .then((body) => {
        if (live) {
          setFeed((feed) => receiveFleet(feed, body, Date.now()));
        }
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const events = new EventSource("/api/events");
    const onFleet = (event: MessageEvent<string>) => {
      setFeed((feed) => receiveFleet(feed, event.data, Date.now()));
    };
    const onTick = (event: MessageEvent<string>) => {
      setFeed((feed) => receiveTick(feed, event.data, Date.now()));
    };
    const onError = () => {
      setFeed((feed) => streamFailed(feed, DROPPED));
    };
    events.addEventListener("fleet", onFleet as EventListener);
    events.addEventListener("tick", onTick as EventListener);
    events.onerror = onError;
    return () => {
      events.removeEventListener("fleet", onFleet as EventListener);
      events.removeEventListener("tick", onTick as EventListener);
      events.onerror = null;
      events.close();
    };
  }, []);

  return { feed, localNow };
}
