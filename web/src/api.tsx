/** How the dashboard talks to the bot: one request helper and one polling hook, both of which
 * surface failures instead of swallowing them (LESSONS 24: dashboards must not lie). */
import { useEffect, useState } from 'react';
import { time } from './format';

/** GET (or POST with `body`) `/api{path}`; a non-2xx answer throws the server's `detail`. */
export async function request<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { 'Content-Type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const data = await response.json().catch(() => {
    if (response.ok) throw new Error(`Invalid JSON response (${path})`);
    return null;
  });
  if (!response.ok) throw new Error((data as { detail?: string } | null)?.detail || `Request failed (${response.status})`);
  return data as T;
}

export interface Poll<T> {
  /** The last successful answer (kept across a failed poll). */
  data: T | undefined;
  /** Why the latest poll failed, or null when it succeeded. */
  error: string | null;
  /** When `data` was fetched (ms since epoch). */
  updatedAt: number | null;
  /** Poll again now. */
  refresh: () => void;
}

/** Fired once the operator session is established, so every panel polls again at once: a panel
 * that polled before the token was entered would otherwise keep "Operator token required" until its
 * own timer came round (up to five minutes; 2026-09-27). */
export const SESSION_EVENT = 'sv-session';

/** Announce a fresh operator session to every polling panel. */
export function announceSession() {
  window.dispatchEvent(new Event(SESSION_EVENT));
}

/** Poll `path` every `ms` (a null path pauses); a change of `key` also re-polls, and so does a new
 * operator session. */
export function usePoll<T>(path: string | null, ms: number, key?: unknown): Poll<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const again = () => setTick(t => t + 1);
    window.addEventListener(SESSION_EVENT, again);
    return () => window.removeEventListener(SESSION_EVENT, again);
  }, []);
  useEffect(() => {
    if (!path) return;
    let alive = true;
    const load = () => request<T>(path)
      .then(v => { if (alive) { setData(v); setError(null); setUpdatedAt(Date.now()); } })
      .catch((e: unknown) => { if (alive) setError(e instanceof Error ? e.message : String(e)); });
    load();
    const id = window.setInterval(load, ms);
    return () => { alive = false; window.clearInterval(id); };
  }, [path, ms, key, tick]);
  return { data, error, updatedAt, refresh: () => setTick(t => t + 1) };
}

/** A one-line note under a panel whose latest poll failed: what failed and how old the data is. */
export function StaleNote({ poll }: { poll: Pick<Poll<unknown>, 'error' | 'updatedAt' | 'data'> }) {
  if (!poll.error) return null;
  if (poll.error === 'Operator token required') return <p className="stale-note" role="status">Waiting for the operator token; this panel loads as soon as the control room is unlocked.</p>;
  const age = poll.updatedAt ? `showing data from ${time(poll.updatedAt / 1000, { seconds: true })}` : 'no data yet';
  return <p className="stale-note" role="status">Update failed ({poll.error}); {age}.</p>;
}
