import { test, expect } from '@playwright/test';

// The panels under test live on the full board (the All view).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

test('the learner pacing inputs keep the saved values when the snapshot behind them is stale', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    Object.assign(state.training.settings, { cooldown_minutes: 60, min_new_hands: 500 });
    await route.fulfill({ json: state });
  });
  await page.route('**/api/training/settings', route => route.fulfill({ json: { cooldown_minutes: 45, min_new_hands: 300 } }));
  await page.goto('/');
  const form = page.getByRole('form', { name: 'Learner pacing' });
  await expect(form.getByLabel(/cooldown between searches/i)).toHaveValue('60');
  await form.getByLabel(/cooldown between searches/i).fill('45');
  await form.getByLabel(/new hands before a champion search/i).fill('300');
  await form.getByRole('button', { name: 'Save' }).click();
  await expect(form.getByText(/Saved: champion search after 300 new hands/)).toBeVisible();
  // `/api/state` still answers with the old settings, as an SSE snapshot up to 5 s old would.
  await expect(form.getByLabel(/cooldown between searches/i)).toHaveValue('45');
  await expect(form.getByLabel(/new hands before a champion search/i)).toHaveValue('300');
});
