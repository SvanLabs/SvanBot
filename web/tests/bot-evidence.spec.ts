import { test, expect, type Page } from '@playwright/test';
import type { Snapshot } from '../src/types';

async function twoBots(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('svan-view', 'all');
    window.EventSource = class {
      onopen: (() => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  await page.route('**/api/state', async route => {
    const state: Snapshot = await (await route.fetch()).json();
    state.bots = [state.bots[0], { ...structuredClone(state.bots[0]), slot: 1, name: 'OtherBot' }];
    await route.fulfill({ json: state });
  });
}

test('switching bots never keeps another bots hands or opponents after a failed read', async ({ page }) => {
  await twoBots(page);
  await page.route('**/api/bots/0/hands', route => route.fulfill({ json: [{ id: 1, hand_id: 'first-hand', ts: 1, hole: ['As', 'Kd'], board: [], net: 20, big_blind: 20, version: '' }] }));
  await page.route('**/api/bots/0/opponents', route => route.fulfill({ json: [{ name: 'FirstOpponent', style: 'Tight', advice: '', evidence_hands: 10 }] }));
  await page.route('**/api/bots/1/hands', route => route.fulfill({ status: 503, json: { detail: 'other hands unavailable' } }));
  await page.route('**/api/bots/1/opponents', route => route.fulfill({ status: 503, json: { detail: 'other opponents unavailable' } }));
  await page.goto('/');
  const hands = page.locator('[data-widget="recent-hands"]');
  const opponents = page.locator('[data-widget="opponents"]');
  await expect(hands).toContainText('#first-han');
  await expect(opponents).toContainText('FirstOpponent');
  await page.getByRole('button', { name: 'OtherBot, offline', exact: true }).click();
  await expect(page.locator('.breadcrumb b')).toHaveText('OtherBot');
  await expect(hands).not.toContainText('#first-han');
  await expect(opponents).not.toContainText('FirstOpponent');
  await expect(hands).toContainText('other hands unavailable');
  await expect(opponents).toContainText('other opponents unavailable');
});

test('an opponent read failure does not hide successfully loaded hands', async ({ page }) => {
  await twoBots(page);
  await page.route('**/api/bots/0/hands', route => route.fulfill({ json: [{ id: 1, hand_id: 'kept-hand', ts: 1, hole: ['As', 'Kd'], board: [], net: 20, big_blind: 20, version: '' }] }));
  await page.route('**/api/bots/0/opponents', route => route.fulfill({ status: 503, json: { detail: 'opponents unavailable' } }));
  await page.goto('/');
  await expect(page.locator('[data-widget="recent-hands"]')).toContainText('#kept-hand');
  await expect(page.locator('[data-widget="opponents"]')).toContainText('opponents unavailable');
});

test('a range read failure is named instead of described as no next decision', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/bots/0/ranges', route => route.fulfill({ status: 503, json: { detail: 'ranges unavailable' } }));
  await page.goto('/');
  await expect(page.locator('[data-widget="ranges"]')).toContainText('ranges unavailable');
});

test('range explorer clears the previous bots range when the selected bot cannot load', async ({ page }) => {
  await twoBots(page);
  await page.route('**/api/bots/0/ranges', route => route.fulfill({ json: {
    available: true, hole: ['As', 'Kd'], board: [], street: 'preflop', pot: 60, to_call: 20, position: 'BTN',
    equity_vs_all: 0.62, range_model: 'first bot model',
    opponents: [{ seat: 1, name: 'FirstRangeOpponent', position: 'BB', grid: Array(169).fill(1), share: Array(169).fill(1 / 169),
      top: [], equity: 0.6, profile: { hands: 120, vpip: 0.31, pfr: 0.22, confidence: 0.8 } }],
  } }));
  await page.route('**/api/bots/1/ranges', route => route.fulfill({ status: 503, json: { detail: 'other ranges unavailable' } }));
  await page.goto('/');
  const ranges = page.locator('[data-widget="ranges"]');
  await expect(ranges).toContainText('FirstRangeOpponent');
  await page.getByRole('button', { name: 'OtherBot, offline', exact: true }).click();
  await expect(ranges).not.toContainText('FirstRangeOpponent');
  await expect(ranges).toContainText('other ranges unavailable');
});
