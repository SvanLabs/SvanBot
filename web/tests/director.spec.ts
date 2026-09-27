import { expect, test } from '@playwright/test';
import { commentary, directorScore, MIN_SHOT_MS, nextShot } from '../src/director';
import type { Bot } from '../src/types';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

const bot = (slot: number, over: Partial<Bot> = {}): Bot => ({
  slot, name: `b${slot}`, connected: true, hand_id: `h${slot}`, big_blind: 20, pot: 200, actor_seat: 3, hero_seat: 0,
  seats: [{ seat: 0, name: 'us', stack: 2000 }, { seat: 3, name: 'v', stack: 2000 }],
  ...over,
} as unknown as Bot);

test('the director prefers our turn, then all-ins, then big pots', () => {
  const ours = bot(0, { actor_seat: 0 });
  const allIn = bot(1, { seats: [{ seat: 0, name: 'us', stack: 0 }, { seat: 3, name: 'v', stack: 0, last_action: 'all_in' }] });
  const bigPot = bot(2, { pot: 20_000 });
  const idle = bot(3, { hand_id: null as unknown as string });
  expect(directorScore(ours)).toBeGreaterThan(directorScore(allIn));
  expect(directorScore(allIn)).toBeGreaterThan(directorScore(bot(4)));
  expect(directorScore(bigPot)).toBeGreaterThan(directorScore(bot(4)));
  expect(directorScore(idle)).toBeLessThan(0);
});

test('the director holds a shot before cutting and leaves a finished table at once', () => {
  const bots = [bot(0), bot(1, { actor_seat: 0 })];
  expect(nextShot(bots, 0, 1000)).toBe(0);
  expect(nextShot(bots, 0, MIN_SHOT_MS)).toBe(1);
  const ended = [bot(0, { hand_id: null as unknown as string }), bot(1)];
  expect(nextShot(ended, 0, 10)).toBe(1);
});

test('commentary speaks in plain lines', () => {
  expect(commentary({ type: 'decision', bot: 'Svanar', action: 'all_in', equity: 0.71 })).toBe('Svanar moves ALL IN at 71%!');
  expect(commentary({ type: 'result', bot: 'Svanar', net: 1200 })).toBe('Svanar drags the pot: +1,200 chips!');
  expect(commentary({ type: 'board' })).toBeNull();
});
