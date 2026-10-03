import { test, expect, type Page } from '@playwright/test';

// Reading `localStorage` is not a total function (issue #7). Where the origin is opaque or site data
// is blocked — a private window with storage denied, a sandboxed iframe, a `file://` page — the
// accessor itself raises `SecurityError` instead of returning `null`. The dashboard reads it in the
// first render of the entry point, in a `useState` initializer, and again in every `Panel`; there is
// no error boundary above either, so a throw unmounts the tree and the operator gets a blank page
// rather than a dashboard.
//
// This forces that throw. Clearing the store instead would exercise the missing-key path, which the
// page already survives: it is the throwing accessor that blanks it.

/** Make every `localStorage` access throw, the way an opaque origin's does. */
async function denyStorage(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() { throw new DOMException('The operation is insecure.', 'SecurityError'); },
    });
  });
}

test('the dashboard renders when every localStorage read throws', async ({ page }) => {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await denyStorage(page);
  await page.goto('/');

  // The shell is there: the view tabs, the board, and a panel inside it — `Panel` holds one of the
  // unguarded reads, so its content is the assertion that the read fell back instead of throwing.
  await expect(page.getByRole('tablist', { name: 'Dashboard views' })).toBeVisible();
  const panel = page.locator('[data-widget="table"]');
  await expect(panel).toBeVisible();
  await expect(panel.locator('.panel-heading h2')).toHaveText('Live table');
  await expect(page.locator('[data-widget="recent-hands"]')).toBeVisible();
  await expect(page.locator('footer')).toContainText('SVANBOT');

  // Each read that runs before the first paint falls back to its documented default, and the page
  // shows it: the remembered view is Live, the layout is not compact, no panel starts collapsed,
  // and the table theme is the default one.
  await expect(page.getByRole('tab', { name: 'Live' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('div.app')).not.toHaveClass(/\bcompact\b/);
  await expect(panel.locator('.panel-content')).toBeVisible();
  await expect(page.locator('.theme-toggle button[aria-pressed="true"]')).toHaveText('Arena');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  // The slot read falls back to no selection, so the first bot in the snapshot is the one shown.
  await expect(page.locator('.breadcrumb b')).not.toHaveText('Control room');

  expect(errors, `uncaught page errors: ${errors.map(error => error.message).join('; ')}`).toHaveLength(0);
});

test('dashboard preferences still change when browser storage throws', async ({ page }) => {
  const errors: Error[] = [];
  page.on('pageerror', error => errors.push(error));
  await denyStorage(page);
  await page.goto('/');
  await page.getByRole('button', { name: 'Felt', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Felt', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'Minimize Live table', exact: true }).click();
  await expect(page.locator('[data-widget="table"] .panel-content')).toBeHidden();
  await page.getByRole('button', { name: 'Expand Live table', exact: true }).click();
  await page.getByRole('button', { name: 'Open settings' }).click();
  await page.getByRole('switch', { name: 'Compact layout' }).click();
  await expect(page.getByRole('switch', { name: 'Compact layout' })).toHaveAttribute('aria-checked', 'true');
  // The theme picker's write is as guarded as every other preference, and the room re-inks this
  // visit even though nothing was remembered.
  await page.getByRole('dialog').getByRole('button', { name: 'Light', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Close settings' }).click();
  await page.locator('.bot-tabs button').first().click();
  expect(errors, `uncaught preference errors: ${errors.map(error => error.message).join('; ')}`).toHaveLength(0);
});
