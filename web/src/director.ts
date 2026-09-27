import type { TableBot } from './types';

/** TV mode auto-director (0177): how interesting a live table is right now. Our bot to act beats an
 * all-in, which beats a big pot; empty or idle tables score below zero. */
export function directorScore(b: TableBot): number {
  if (!b.connected || !b.hand_id) return -1;
  const bb = b.big_blind || 20;
  let s = 0;
  if (b.actor_seat != null && b.actor_seat === b.hero_seat) s += 1000;
  if (b.seats.some(x => x.last_action === 'all_in' && !x.folded)) s += 800;
  s += Math.min((b.pot || 0) / bb, 2000);
  s += b.seats.filter(x => x.name && !x.folded).length * 20;
  return s;
}

/** Minimum time on one table before a cut, unless that table's hand is over. */
export const MIN_SHOT_MS = 8000;

/** Which table to show: stay on `current` until it has been on screen `MIN_SHOT_MS` (or its hand
 * ended), then cut to the best table if it beats the current one clearly. */
export function nextShot(bots: TableBot[], current: number | undefined, shownFor: number): number | undefined {
  const live = bots.map(b => ({ slot: b.slot, score: directorScore(b) })).filter(x => x.score >= 0);
  if (!live.length) return current ?? bots[0]?.slot;
  const best = live.reduce((a, b) => (b.score > a.score ? b : a));
  const cur = live.find(x => x.slot === current);
  if (!cur) return best.slot;
  if (shownFor < MIN_SHOT_MS) return current;
  return best.score > cur.score + 150 ? best.slot : current;
}

/** One commentary line for a live decision or result, in plain broadcast style. */
export function commentary(ev: { type: string; bot?: string; action?: string; amount?: number | null; equity?: number; net?: number | null }): string | null {
  const who = ev.bot ?? 'Svan';
  const eq = ev.equity == null ? null : Math.round(ev.equity * 100);
  if (ev.type === 'decision') {
    switch (ev.action) {
      case 'fold': return `${who} lets it go${eq != null ? ` with only ${eq}% to win` : ''}.`;
      case 'check': return `${who} checks it back${eq != null ? ` at ${eq}%` : ''}.`;
      case 'call': return `${who} calls${eq != null ? `: ${eq}% to win, and the price is right` : ''}.`;
      case 'raise': return `${who} raises${ev.amount ? ` to ${ev.amount.toLocaleString('en-US')}` : ''}${eq != null ? ` with ${eq}% equity` : ''}!`;
      case 'all_in': return `${who} moves ALL IN${eq != null ? ` at ${eq}%` : ''}!`;
      default: return null;
    }
  }
  if (ev.type === 'result' && ev.net != null && ev.net !== 0) {
    return ev.net > 0 ? `${who} drags the pot: +${ev.net.toLocaleString('en-US')} chips!` : `${who} gives up ${Math.abs(ev.net).toLocaleString('en-US')} chips. On to the next one.`;
  }
  return null;
}
