import { test, expect } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

// Bot setup against the sandboxed backend: one dry-run bot from the process environment, no .env
// file, loopback host (saving allowed without a token), not supervised (no automatic restart), and
// every openpoker.ai endpoint pointed at a closed port.

test('bot setup lists bots without keys, validates, checks keys and saves', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', {name:'Open settings'}).click();
  await page.getByRole('link', {name:'Open bot setup'}).click();
  await expect(page.getByRole('heading', {name:'Bots and table settings'})).toBeVisible();
  await expect(page.getByLabel('Bot 1 name')).toHaveValue('TestBot');
  // Only a hint of the stored key reaches the browser.
  await expect(page.getByLabel('Bot 1 API key')).toHaveAttribute('placeholder', /Stored key …-key/);
  const state = await (await page.request.get('/api/setup')).text();
  expect(state).not.toContain('test-key');

  // A key check reports that openpoker.ai is unreachable instead of pretending.
  await page.getByRole('button', {name:'Check key'}).click();
  await expect(page.getByText('openpoker.ai is unreachable; the key was not checked')).toBeVisible();

  // Validation is server-side and explained: the sandbox key is too short to be real.
  await page.getByRole('button', {name:'Save and apply'}).click();
  await expect(page.getByRole('alert')).toContainText('Bot 1: that does not look like an API key.');

  await page.getByLabel('Bot 1 API key').fill('op_test_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1111');
  await page.getByRole('button', {name:'Add bot'}).click();
  await page.getByLabel('Bot 2 name').fill('Second_Bot');
  await page.getByLabel('Bot 2 API key').fill('op_test_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb2222');
  await page.getByRole('switch', {name:'Second_Bot plays'}).click();
  await page.getByLabel('Maximum buy-in').fill('3000');
  await page.getByRole('button', {name:'Save and apply'}).click();
  await expect(page.getByRole('status')).toContainText('Saved to .env. Restart the fleet');
  // Reloaded from the server: the new bot is there, switched off, with only its hint.
  await expect(page.getByLabel('Bot 2 name')).toHaveValue('Second_Bot');
  await expect(page.getByLabel('Bot 2 API key')).toHaveAttribute('placeholder', /Stored key …2222/);
  await expect(page.getByRole('switch', {name:'Second_Bot plays'})).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByLabel('Maximum buy-in')).toHaveValue('3000');
});
