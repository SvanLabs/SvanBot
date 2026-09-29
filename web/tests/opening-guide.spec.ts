import { test, expect } from '@playwright/test';

test('opening guide reports failed reads and recovers when the operator session opens', async ({ page }) => {
  let ready = false;
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/starting-hands', route => ready
    ? route.fulfill({ json: [{ hand: 'AA', score: 1, open: [true, true, true, true, true, true] }] })
    : route.fulfill({ status: 503, json: { detail: 'opening guide unavailable' } }));
  await page.goto('/');
  const panel = page.locator('[data-widget="starting-hands"]');
  await expect(panel).toContainText('opening guide unavailable');
  ready = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel.locator('.range-grid > span')).toHaveText('AA');
  await expect(panel).not.toContainText('opening guide unavailable');
});
