import { test, expect } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

test('results monitor widget reports the monitor, pressure, replays and alerts honestly', async ({ page }) => {
  await page.goto('/');
  const panel = page.locator('section.panel', { has: page.getByRole('heading', { name: 'Results monitor' }) });
  await expect(panel).toBeVisible();
  // The sandbox has no monitor log: it must say so rather than show stale data.
  await expect(panel.getByText('Not writing — check scripts/start.sh')).toBeVisible();
  await expect(panel.getByText(/CPU .*% · I\/O .*% · mem/)).toBeVisible();
  await expect(panel.getByText('0 (kept 14 days)')).toBeVisible();

  // With alerts and summaries served, they render with their kinds.
  await page.route('**/api/monitor', route => route.fulfill({ json: {
    monitor: { running: true, log_age_seconds: 60, started: null,
      summary: { time: '12:00', kind: 'SUMMARY', text: '30m: 216 hands +27449 chips | SurSvan +13130/45 | we lost most to: Glow -299bb' },
      opponents: { time: '12:00', kind: 'OPPONENTS', text: '18 faced, 2 new: A, B | most played: Glow (station VPIP 52/PFR 9, 400h, -299bb/40)' },
      alerts: [{ time: '11:40', kind: 'BIGWIN', text: 'SurSvan +9000 chips (+450 bb)' }, { time: '11:10', kind: 'NEMESIS', text: 'Glow beats us' }] },
    pressure: { cpu: 3.2, io: 0.4, memory: 0 }, replays: { recorded: 212, newest: null, keep_days: 14 },
    season_check: { result: 'season check season12: PASSED', failures: [] } } }));
  await page.reload();
  await expect(panel.getByText('Running')).toBeVisible();
  await expect(panel.getByText('BIGWIN')).toBeVisible();
  await expect(panel.getByText(/most played: Glow \(station/)).toBeVisible();
  await expect(panel.getByText('season check season12: PASSED')).toBeVisible();
});

test('an unreadable replay store says unreadable, never 0 (0326)', async ({ page }) => {
  await page.route('**/api/monitor', route => route.fulfill({ json: {
    monitor: { running: true, log_age_seconds: 60, started: null, summary: null, opponents: null, alerts: [] },
    pressure: { cpu: 3.2, io: 0.4, memory: 0 },
    replays: { recorded: null, error: 'store unreadable (replays): database is locked', newest: null, keep_days: 14 },
    season_check: null } }));
  await page.goto('/');
  const panel = page.locator('section.panel', { has: page.getByRole('heading', { name: 'Results monitor' }) });
  await expect(panel.getByText('unreadable — the replay store read failed')).toBeVisible();
  await expect(panel.getByText('unreadable — the replay store read failed')).toHaveAttribute('title', 'store unreadable (replays): database is locked');
  await expect(panel.getByText(/0 \(kept 14 days\)/)).toHaveCount(0);
});
