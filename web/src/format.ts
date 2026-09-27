/** Number and card formatting shared by every panel. */

/** A number with at most `d` decimals (exactly `d` when `fixed`), or an em dash when missing. */
export const fmt = (v: number | null | undefined, d = 0, fixed = false) =>
  v == null || !isFinite(v) ? '—' : v.toLocaleString('en-US', { maximumFractionDigits: d, ...(fixed ? { minimumFractionDigits: d } : {}) });

/** A signed number: `+12`, `-3`, `0`. */
export const sgn = (v: number | null | undefined, d = 0, fixed = false) =>
  v == null || !isFinite(v) ? '—' : `${v > 0 ? '+' : ''}${fmt(v, d, fixed)}`;

/** A share (0.25) as a percentage (`25%`). */
export const pct = (v: number | null | undefined, d = 0) => v == null || !isFinite(v) ? '—' : `${fmt(v * 100, d)}%`;

/** A share that keeps a decimal while it is small, so 0.0553 reads `5.5%` and 0.62 reads `62%`. */
export const share = (v: number | null | undefined) => v == null || !isFinite(v) ? '—' : pct(v, Math.abs(v * 100) < 10 ? 1 : 0);

/** Suit letters as symbols. */
export const SUITS: Record<string, string> = { h: '♥', d: '♦', s: '♠', c: '♣' };

/** The server auto-folds or auto-checks 45 s after it sends `your_turn` (the protocol's turn
 *  timeout; `crates/apps/bot/src/api/state.rs` TURN_DEADLINE_MS). Decisions take a fraction of it,
 *  and the countdown in the table heading counts the same clock. */
export const TURN_DEADLINE_S = 45;

/* ------------------------------------------------------------------------------------------------
 * The clock (0295). Every visible time on the dashboard goes through `time`, and only the browser
 * formats it: the API sends unix seconds (or an RFC 3339 string with an offset) and nothing else,
 * so the server's zone — or a UTC label invented in the UI — can never disagree with the browser's
 * clock. Resolved once, in the viewer's own zone as `Intl` reports it.
 * ---------------------------------------------------------------------------------------------- */
const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;
/** The day the viewer's zone is in at a moment, as `2026-09-27`, so "today" is the viewer's today. */
const dayIn = new Intl.DateTimeFormat('en-CA', { timeZone: ZONE, year: 'numeric', month: '2-digit', day: '2-digit' });
const yearIn = new Intl.DateTimeFormat('en-GB', { timeZone: ZONE, year: 'numeric' });
const clockIn = (seconds: boolean) => new Intl.DateTimeFormat('en-GB', { timeZone: ZONE, hourCycle: 'h23', hour: '2-digit', minute: '2-digit', ...(seconds ? { second: '2-digit' } : {}) });
const CLOCK = clockIn(false);
const CLOCK_S = clockIn(true);
const DAY = new Intl.DateTimeFormat('en-GB', { timeZone: ZONE, day: 'numeric', month: 'short' });
const DAY_YEAR = new Intl.DateTimeFormat('en-GB', { timeZone: ZONE, day: 'numeric', month: 'short', year: 'numeric' });
const WEEKDAY = new Intl.DateTimeFormat('en-GB', { timeZone: ZONE, weekday: 'short' });

/** A stamp in seconds, or an RFC 3339 string the API sent (with an offset, or `Z`). */
export type Stamp = number | string | null | undefined;

/** `seconds` — a unix stamp, or a string of digits that is one; otherwise an RFC 3339 date-time,
 *  read as the browser's local time when it carries no offset (the best reading available). */
const moment = (stamp: Stamp): Date | null => {
  if (stamp == null) return null;
  if (typeof stamp === 'string') {
    const text = stamp.trim();
    if (!text) return null;
    const ms = /^\d+(\.\d+)?$/.test(text) ? Number(text) * 1000 : Date.parse(text);
    return isFinite(ms) ? new Date(ms) : null;
  }
  return isFinite(stamp) ? new Date(stamp * 1000) : null;
};

export interface ClockStyle {
  /** Add seconds (log lines, replay events). */
  seconds?: boolean;
  /** Show the date even when the moment is today (releases, saved builds). */
  date?: boolean;
  /** The date alone, no time of day (a season's start). */
  dateOnly?: boolean;
  /** Lead with the weekday — `Sat, 14:03` today, `Fri 26 Sep, 14:03` before that (stories). */
  weekday?: boolean;
}

/** One time, one way, in the viewer's zone: `14:03` while the moment is today; `26 Sep 14:03` when
 *  it is not, with the year when that differs too; `—` when there is no usable stamp. */
export const time = (stamp: Stamp, style: ClockStyle = {}): string => {
  const at = moment(stamp);
  if (!at) return '—';
  const now = new Date();
  const today = dayIn.format(at) === dayIn.format(now);
  const day = (yearIn.format(at) === yearIn.format(now) ? DAY : DAY_YEAR).format(at);
  const clock = (style.seconds ? CLOCK_S : CLOCK).format(at);
  if (style.dateOnly) return day;
  if (style.weekday) return today ? `${WEEKDAY.format(at)}, ${clock}` : `${WEEKDAY.format(at)} ${day}, ${clock}`;
  return today && !style.date ? clock : `${day} ${clock}`;
};
