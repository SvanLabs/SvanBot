import { test, expect } from '@playwright/test';

// Host check (0241): read-only facts with the operator's command for anything off.
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'system')); });

test('the host check names what is off and the command that fixes it', async ({ page }) => {
  await page.route('**/api/host', route => route.fulfill({ json: { checked_at: Date.now() / 1000, checks: [
    { key: 'cpu', label: 'CPU', value: 'Intel(R) Core(TM) i7-4770K CPU @ 3.50GHz · AVX2', status: 'info', advice: null },
    { key: 'microcode', label: 'CPU microcode', value: '0x27 (current 0x28)', status: 'warn', advice: 'sudo apt install intel-microcode (enable non-free-firmware in the APT sources), then reboot' },
    { key: 'thp', label: 'Transparent huge pages', value: 'always', status: 'ok', advice: null },
    { key: 'ssd', label: "Free space (databases' disk)", value: '25.0 GiB', status: 'ok', advice: null },
  ] } }));
  await page.goto('/');
  const panel = page.locator('.host-panel');
  await expect(panel.getByText('1 thing to look at')).toBeVisible();
  await expect(panel.locator('li.host-warn')).toContainText('0x27 (current 0x28)');
  await expect(panel.locator('li.host-warn code')).toContainText('sudo apt install intel-microcode');
  await expect(panel.locator('li.host-ok')).toHaveCount(2);
  await expect(panel.locator('code')).toHaveCount(1);
});

test('the real server answers the host check', async ({ page }) => {
  await page.goto('/');
  const panel = page.locator('.host-panel');
  await expect(panel.locator('li', { hasText: 'CPU' }).first()).toBeVisible();
  await expect(panel.locator('.host-summary')).toBeVisible();
});
