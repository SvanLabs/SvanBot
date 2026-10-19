import { expect, test } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

// 0318: the bias column is one kind of value — the correction in use, two decimals on every row —
// and the state that used to replace it ("learning" / "calibrated") lives in the tooltip, worded
// from the backend's own `bound_by`.
test('the bias column reads a number on every row, and the state is in the tooltip', async ({ page }) => {
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  await page.route('**/api/calibration', route => route.fulfill({json:{
    rows:[
      {category:'flop:bet:big', n:12, predicted_bb:0.4, realized_bb:0.1, residual_bb:-0.3, se_bb:0.31, bias_bb:0, bound_by:'too few samples'},
      {category:'river:call', n:900, predicted_bb:-0.2, realized_bb:-0.44, residual_bb:-0.24, se_bb:0.05, bias_bb:-0.043, bound_by:'evidence, in full'},
      {category:'turn:check', n:700, predicted_bb:0.2, realized_bb:0.21, residual_bb:0.01, se_bb:0.04, bias_bb:0, bound_by:'inside its 95% band'},
    ],
    active_corrections:1,
  }}));
  await page.goto('/');

  const cells = page.locator('[data-widget="calibration"] .calib-bias');
  await expect(cells).toHaveCount(3);
  await expect(cells.nth(0)).toHaveText('0.00 bb');
  await expect(cells.nth(1)).toHaveText('-0.04 bb');
  await expect(cells.nth(2)).toHaveText('0.00 bb');
  // The state still reaches the reader, as the reason behind the zero.
  await expect(cells.nth(0)).toHaveAttribute('title', /no correction: too few decisions to measure yet/);
  await expect(cells.nth(2)).toHaveAttribute('title', /no correction: the gap is inside its 95% band/);
  await expect(cells.nth(1)).toHaveAttribute('title', /applied, size set by evidence, in full/);
});

// #986: the panel shows the 14 busiest classes, then every smaller one with a correction in use or a
// pot-depth split, so a correction the bot applies is never cut off the list.
test('a smaller class with a correction or a deep split is listed past the top 14', async ({ page }) => {
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  const row = (category: string, n: number, bias_bb = 0) =>
    ({category, n, predicted_bb:0, realized_bb:0, residual_bb:0, se_bb:0.1, bias_bb, bound_by:bias_bb ? 'evidence, in full' : 'inside its 95% band'});
  const busy = Array.from({length: 14}, (_, i) => row(`busy:${i}`, 5000 - i));
  await page.route('**/api/calibration', route => route.fulfill({json:{
    rows:[...busy, row('river:call', 400, -15), row('turn:check:small', 300), row('turn:call:deep', 200), row('flop:raise:small', 100)],
    active_corrections:1,
  }}));
  await page.goto('/');

  const names = page.locator('[data-widget="calibration"] .calib-cat');
  await expect(names).toHaveCount(16);
  await expect(names.nth(14)).toContainText('river · call');
  await expect(names.nth(15)).toContainText('turn · call · deep');
  await expect(page.locator('[data-widget="calibration"]')).not.toContainText('turn · check · small');
  await expect(page.locator('[data-widget="calibration"]')).not.toContainText('flop · raise · small');
});
