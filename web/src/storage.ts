/** Browser storage, read the one way that cannot take the page down with it. */

/** The value stored under `key`, or `null` where the browser will not say.
 *
 *  `localStorage` is not a total function: where the origin is opaque or site data is blocked — a
 *  private window with storage denied, a sandboxed iframe, a `file://` page — the accessor raises
 *  `SecurityError` instead of returning `null`. Several of our reads run in a `useState`
 *  initializer, before the first paint and under no error boundary, so a throw there unmounts the
 *  tree and the operator gets a blank page. Every read goes through here, which makes the next one
 *  safe by default rather than by remembering, and costs a remembered preference rather than the
 *  page. */
export const readLocal = (key: string): string | null => {
  try { return localStorage.getItem(key); } catch { return null; }
};

/** Remember a preference where storage permits it; callers keep their current session state. */
export const writeLocal = (key: string, value: string): void => {
  try { localStorage.setItem(key, value); } catch { /* storage unavailable: preference lasts this visit */ }
};
