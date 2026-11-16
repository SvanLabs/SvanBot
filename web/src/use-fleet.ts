"use client";

// The page's one live connection (V11-11): an initial fleet fetch, then the dashboard's event
// stream, with a local clock ticking once a second so ages advance between messages.

import { useEffect, useState } from "react";

import { emptyView, receiveFleet, receiveTick, streamFailed, type View } from "./view";

const CLOCK_MS = 1_000;
const DROPPED = "the event stream dropped; retrying";

export interface LiveView {
  view: View;
  /** This browser's clock, updated once a second. */
  localNow: number;
}

export function useFleet(): LiveView {
  const [view, setView] = useState<View>(emptyView);
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
          setView((view) => receiveFleet(view, body, Date.now()));
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
      setView((view) => receiveFleet(view, event.data, Date.now()));
    };
    const onTick = (event: MessageEvent<string>) => {
      setView((view) => receiveTick(view, event.data, Date.now()));
    };
    const onError = () => {
      setView((view) => streamFailed(view, DROPPED));
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

  return { view, localNow };
}
