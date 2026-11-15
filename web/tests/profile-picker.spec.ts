import { test, expect } from '@playwright/test';

// The panels under test live on the full board (the All view).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });
test('a saved compute profile does not hide a later change the poll reports', async ({ page }) => {
  const preset = (name: string, live_scale: number, learner_threads: number) => ({ name, live_scale, learner_threads, analyst_threads: learner_threads });
  const presets = [preset('quiet', 1, 1), preset('balanced', 1, 4), preset('max', 1, 8)];
  const profile = (active: ReturnType<typeof preset>) => ({ active, stored: true, presets, logical_cores: 8, min_live_scale: 1 });
  let served = profile(presets[2]);
  await page.clock.install();
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/compute', async route => {
    const decisions = { n: 0, p50: null, p95: null, p99: null, max: null };
    await route.fulfill({ json: { decisions, deadline_ms: 45_000, max_share_of_deadline: null, live_samples: 0, deal_chunks: 0, logical_cores: 8, load_average: null, profile: served } });
  });
  await page.route('**/api/compute/profile', async route => {
    served = profile(presets[0]);
    await route.fulfill({ json: served });
  });
  await page.goto('/');
  const buttons = page.getByRole('group', { name: 'Compute profile' });
  await expect(buttons.locator('button.active')).toHaveText('max');
  await buttons.getByRole('button', { name: 'quiet' }).click();
  await expect(buttons.locator('button.active')).toHaveText('quiet');
  // Someone else changes the profile (the CLI, another dashboard); the next poll must show it.
  served = profile(presets[1]);
  await page.clock.runFor(31_000);
  await expect(buttons.locator('button.active')).toHaveText('balanced');
});
