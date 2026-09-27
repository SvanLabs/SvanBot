import { test, expect } from '@playwright/test';

// Dashboard views (0237): the page opens on Live instead of one long scroll, each tab shows its own
// widgets in the user's arrangement, the choice is remembered, and arranging shows every widget.
test('the dashboard opens on Live and each view shows its own widgets', async ({ page }) => {
  await page.goto('/');
  const tabs = page.getByRole('tablist', { name: 'Dashboard views' });
  await expect(tabs.getByRole('tab', { name: 'Live' })).toHaveAttribute('aria-selected', 'true');
  const widget = (id: string) => page.locator(`[data-widget="${id}"]`);
  await expect(widget('table')).toBeVisible();
  await expect(widget('autonomy')).toHaveCount(0);
  await expect(widget('updates')).toHaveCount(0);

  await tabs.getByRole('tab', { name: 'Learning' }).click();
  await expect(widget('autonomy')).toBeVisible();
  await expect(widget('calibration')).toBeVisible();
  await expect(widget('table')).toHaveCount(0);

  await tabs.getByRole('tab', { name: 'System' }).click();
  await expect(widget('updates')).toBeVisible();

  // Remembered across a reload.
  await page.reload();
  await expect(page.getByRole('tab', { name: 'System' })).toHaveAttribute('aria-selected', 'true');
  await expect(widget('updates')).toBeVisible();

  // Arrow keys move between views.
  await page.getByRole('tab', { name: 'System' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
  const all = await page.locator('[data-widget]').count();
  expect(all).toBeGreaterThan(20);
});

test('the Live view is a fraction of the full board in height', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(page.locator('[data-widget="table"]')).toBeVisible();
  const live = await page.evaluate(() => document.documentElement.scrollHeight);
  await page.getByRole('tab', { name: 'All' }).click();
  await expect(page.locator('[data-widget="autonomy"]')).toBeVisible();
  const full = await page.evaluate(() => document.documentElement.scrollHeight);
  expect(live).toBeLessThan(full * 0.75);
});

test('views work at phone width without horizontal scrolling', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page.getByRole('tab', { name: 'Opponents' }).click();
  await expect(page.locator('[data-widget="opponents"]')).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});
