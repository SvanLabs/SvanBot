import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
});

test('decision accuracy names failed reads and repolls after the operator session opens', async ({ page }) => {
  let ready = false;
  await page.route('**/api/accuracy', route => ready
    ? route.fulfill({ json: { days: 7, fleet: { decisions: 0 }, bots: [], streets: [], worst: [] } })
    : route.fulfill({ status: 503, json: { detail: 'accuracy store unavailable' } }));
  await page.goto('/');
  const panel = page.locator('[data-widget="accuracy"]');
  await expect(panel).toContainText('accuracy store unavailable');
  await expect(panel).not.toContainText('Loading decision grades');
  ready = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel).toContainText('No big decision has been re-solved');
  await expect(panel).not.toContainText('accuracy store unavailable');
});

test('decision accuracy keeps a successful reading and labels a failed refresh stale', async ({ page }) => {
  let failed = false;
  await page.route('**/api/accuracy', route => failed
    ? route.fulfill({ status: 503, json: { detail: 'accuracy refresh failed' } })
    : route.fulfill({ json: { days: 7, fleet: { decisions: 10, accuracy: 98.4, grades: [10, 0, 0, 0, 0], mean_loss_bb: 0 }, bots: [], streets: [], worst: [] } }));
  await page.goto('/');
  const panel = page.locator('[data-widget="accuracy"]');
  await expect(panel.locator('.gr-hero')).toContainText('98.4');
  failed = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel).toContainText('accuracy refresh failed');
  await expect(panel).toContainText('showing data from');
  await expect(panel.locator('.gr-hero')).toContainText('98.4');
});
