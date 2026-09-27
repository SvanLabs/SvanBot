import { test, expect } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

test('dashboard shows neural model status', async ({ page }) => {
  await page.goto('/');
  // The engine-check row, not any text mentioning "neural": the activity log can also say
  // "neural response model inactive" at startup, which made a bare /neural/i match ambiguous.
  const row = page.locator('.engine-checks > span').filter({ hasText: /^Neural EV/ });
  await expect(row).toBeVisible();
  await expect(row).toContainText(/ACTIVE|INACTIVE|AVAILABLE|UNAVAILABLE/);
});
