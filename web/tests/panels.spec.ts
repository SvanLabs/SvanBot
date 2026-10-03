import { test, expect, type Page } from '@playwright/test';
import type { DashboardLayout } from '../src/types';

// Panel opening and closing, and the board's arrangement (#729): the whole panel bar toggles collapse
// while the info button does not, the workspace collapses the current view's panels at once, and the
// arrangement the Arrange mode produces is stored on the server with the browser as its fallback.

const panel = (page: Page, title: string) =>
  page.locator('section.panel').filter({ has: page.getByRole('heading', { name: title, exact: true }) });

/** The arrangement the sandbox's server would have stored, in the shape `DashboardLayout` names. */
const CUSTOM: DashboardLayout = { left: ['health'], center: ['table'], right: [], hidden: ['updates'] };

test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

test('the whole panel bar toggles collapse and the info button does not', async ({ page }) => {
  await page.route('**/api/layout', route => route.fulfill({ json: null }));
  await page.goto('/');
  const highlights = panel(page, 'Highlights');
  await expect(highlights.locator('.panel-content')).toBeVisible();

  // The bar itself — not just the chevron — is the toggle.
  await highlights.locator('.panel-heading').click();
  await expect(highlights).toHaveClass(/panel-collapsed/);
  await expect(highlights.locator('.panel-content')).toBeHidden();
  await highlights.locator('.panel-heading').click();
  await expect(highlights.locator('.panel-content')).toBeVisible();

  // The info button explains the panel without closing it.
  await highlights.getByRole('button', { name: 'About Highlights' }).click();
  await expect(highlights.getByRole('region', { name: 'Highlights explanation' })).toBeVisible();
  await expect(highlights).not.toHaveClass(/panel-collapsed/);

  // The chevron stays the explicit affordance, with its state on the button.
  await highlights.getByRole('button', { name: 'Minimize Highlights' }).click();
  await expect(highlights).toHaveClass(/panel-collapsed/);
  await highlights.getByRole('button', { name: 'Expand Highlights' }).click();
  await expect(highlights.locator('.panel-content')).toBeVisible();
});

test('a panel collapse saved under the old title key still applies, and is written back by id', async ({ page }) => {
  // Before #729 the key was the title string; the widget id took over, with the old key as fallback.
  await page.addInitScript(() => localStorage.setItem('svan-panel-collapsed:Live table', 'true'));
  await page.route('**/api/layout', route => route.fulfill({ json: null }));
  await page.goto('/');
  const table = panel(page, 'Live table');
  await expect(table).toHaveClass(/panel-collapsed/);
  await table.getByRole('button', { name: 'Expand Live table' }).click();
  expect(await page.evaluate(() => localStorage.getItem('svan-panel-collapsed:table'))).toBe('false');
});

test('Collapse all acts on the current view, and Expand all undoes it', async ({ page }) => {
  await page.route('**/api/layout', route => route.fulfill({ json: null }));
  await page.goto('/');
  await page.getByRole('tab', { name: 'Learning' }).click();

  await page.getByRole('button', { name: 'Collapse all' }).click();
  await expect(page.getByRole('button', { name: 'Expand all' })).toBeVisible();
  const learning = ['autonomy', 'experiments', 'calibration', 'accuracy', 'wiring'];
  for (const id of learning) await expect(page.locator(`[data-widget="${id}"] section.panel`)).toHaveClass(/panel-collapsed/);

  // A panel of another view was not touched: the control acts on what this view shows.
  await page.getByRole('tab', { name: 'System' }).click();
  await expect(page.locator('[data-widget="updates"] section.panel')).not.toHaveClass(/panel-collapsed/);
  await expect(page.getByRole('button', { name: 'Collapse all' })).toBeVisible();

  // The Learning view kept what the control did to it, and expands in one click.
  await page.getByRole('tab', { name: 'Learning' }).click();
  await page.getByRole('button', { name: 'Expand all' }).click();
  await expect(page.locator('[data-widget="autonomy"] section.panel')).not.toHaveClass(/panel-collapsed/);
  await expect(page.getByRole('button', { name: 'Collapse all' })).toBeVisible();
});

test('a stored server layout is what ordinary loads render', async ({ page }) => {
  await page.route('**/api/layout', route => route.fulfill({ json: CUSTOM }));
  await page.goto('/');
  await expect(page.locator('.dashboard-grid')).toHaveClass(/custom-layout/);
  await expect(page.locator('[data-column="left"] [data-widget]').first()).toHaveAttribute('data-widget', 'health');
  await expect(page.locator('[data-widget="updates"]')).toHaveCount(0);
  // Cached for the next load, which is the fallback when the endpoint cannot be reached.
  expect(await page.evaluate(() => localStorage.getItem('svan-layout:v1'))).toContain('"health"');
});

test('Arrange mode writes the arrangement through to the endpoint', async ({ page }) => {
  let posted: DashboardLayout | undefined;
  await page.route('**/api/layout', route => {
    if (route.request().method() === 'GET') return route.fulfill({ json: null });
    posted = route.request().postDataJSON() as DashboardLayout;
    return route.fulfill({ json: posted });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Arrange widgets' }).click();
  await page.getByRole('button', { name: 'Move Highlights down' }).click();
  await expect.poll(() => posted?.left.join(',')).toBe('autonomy,experiments,experiment-mode,calibration,accuracy,wiring,health,highlights,privacy');
});

test('the browser-local layout is the fallback when the endpoint is unavailable', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-layout:v1', JSON.stringify({ left: ['health'], center: ['table'], right: [], hidden: ['updates'] })));
  await page.route('**/api/layout', route => route.fulfill({ status: 503, json: { detail: 'layout store unavailable' } }));
  await page.goto('/');
  await expect(page.locator('.dashboard-grid')).toHaveClass(/custom-layout/);
  await expect(page.locator('[data-column="left"] [data-widget]').first()).toHaveAttribute('data-widget', 'health');
  // Arranging still works with the server down: the change lands in this browser.
  await page.getByRole('button', { name: 'Arrange widgets' }).click();
  await page.getByRole('button', { name: 'Move Highlights down' }).click();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('svan-layout:v1')!) as DashboardLayout);
  expect(stored.left.indexOf('highlights')).toBeGreaterThan(stored.left.indexOf('privacy'));
});

// The endpoint itself, unmocked. The sandbox's store is shared by every test in the run, so this
// test stores an arrangement and clears it again (never relies on either being there), and pins the
// server contract rather than a page render — the render is covered above with a mocked answer.
test('the layout endpoint round-trips an arrangement and clears it', async ({ request }) => {
  // A `null` document is "none stored". Playwright sends a raw string body as octet-stream, so the
  // JSON content type is explicit here.
  const clear = () => request.post('/api/layout', { headers: { 'Content-Type': 'application/json' }, data: 'null' });
  try {
    await clear();
    expect(await (await request.get('/api/layout')).json()).toBeNull();
    // A body that is not an arrangement is refused and stores nothing.
    expect((await request.post('/api/layout', { data: { left: 'health' } })).status()).toBe(400);
    expect(await (await request.get('/api/layout')).json()).toBeNull();
    const saved = await request.post('/api/layout', { data: CUSTOM });
    expect(saved.ok()).toBe(true);
    expect(await saved.json()).toEqual(CUSTOM);
    expect(await (await request.get('/api/layout')).json()).toEqual(CUSTOM);
  } finally {
    await clear();
  }
});
