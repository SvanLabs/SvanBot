import { test, expect } from '@playwright/test';

test('a malformed successful API reply is a failed read and keeps the dashboard usable', async ({ page }) => {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/intel', route => route.fulfill({ status: 200, contentType: 'application/json', body: '{broken' }));
  await page.goto('/');
  await expect(page.locator('[data-widget="intel"]')).toContainText('Invalid JSON response');
  await expect(page.getByRole('tablist', { name: 'Dashboard views' })).toBeVisible();
  expect(errors).toHaveLength(0);
});

test('an invalid JSON refresh preserves the last successful panel reading', async ({ page }) => {
  let broken = false;
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/intel', route => broken
    ? route.fulfill({ status: 200, contentType: 'application/json', body: '{broken' })
    : route.fulfill({ json: { fits: [], opponents: [], corrected_opponents: 4 } }));
  await page.goto('/');
  const panel = page.locator('[data-widget="intel"]');
  await expect(panel).toContainText('4 in all');
  broken = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel).toContainText('Invalid JSON response');
  await expect(panel).toContainText('showing data from');
  await expect(panel).toContainText('4 in all');
});
