import { test, expect } from '@playwright/test';

test('Results timeline connects a silent interval, a release, a dip and exact hand drill-down', async ({ page }) => {
  const start = Date.parse('2026-09-22T00:00:00Z') / 1000;
  const hours = Array.from({ length: 96 }, (_, i) => ({
    ts: start + i * 3600,
    hands: i >= 2 && i <= 13 ? 0 : 1,
    priced: i >= 2 && i <= 13 ? 0 : 1,
    net: i >= 2 && i <= 13 ? 0 : i >= 72 ? -8 : 12,
    ev_net: i >= 2 && i <= 13 ? 0 : i >= 72 ? -6 : 10,
    ev_net_sq: i >= 2 && i <= 13 ? 0 : i >= 72 ? 36 : 100,
  }));
  await page.route('**/api/timeline', route => route.fulfill({ json: {
    scope: { scoped: true, number: 13 }, from: start, until: start + 96 * 3600,
    window_limited: false, release_log_available: true, operations_ledger_available: true, hours, updated: start + 96 * 3600,
    marks: [
      { id: 'release:keepalive', kind: 'release', ts: start + 14 * 3600, title: 'Release abc1234', detail: 'Install five-minute keepalive', source: { line: 12, commit: 'abc1234', subject: 'Install five-minute keepalive' } },
      { id: 'operation:1', kind: 'operation', ts: start + 19 * 3600 + 21 * 60, title: 'Five-minute keepalive enabled', detail: 'Operator enabled timer', source: { ledger_line: 1, record: { ticket: '0145', section: 'Installed (2026-09-22 19:21 UTC)' } } },
    ],
  } }));
  await page.route('**/api/timeline/hour/*', route => {
    const hour = Number(route.request().url().split('/').pop());
    const item = hours.find(h => h.ts === hour);
    return route.fulfill({ json: { hour, hands: item?.hands ? [{ slot: 0, bot: 'TestBot', stored_bot: 'TestBot', hand_id: 'hand-1', ts: hour + 60, net: item.net, ev_net: item.ev_net }] : [] } });
  });
  await page.route('**/api/bots/0/hands/hand-1', route => route.fulfill({ json: [
    { type: 'hand_start', ts: start, data: { hole_cards: ['As', 'Kd'], players: [] } },
    { type: 'hand_result', ts: start, data: { board: [], net: 12 } },
  ] }));

  await page.goto('/');
  await page.getByRole('tab', { name: 'Results' }).click();
  const axis = page.getByRole('heading', { name: 'Unified event timeline' }).locator('..').locator('..');
  await expect(axis.getByLabel('Scrub season by hour')).toBeVisible();
  await axis.getByLabel('Jump to UTC date').fill('2026-09-22');
  await axis.getByRole('button', { name: /03:00 UTC: 0 hands/ }).click();
  await expect(axis.getByText('No stored hands in this hour.')).toBeVisible();
  await axis.getByRole('button', { name: /14:00 UTC: 1 hands/ }).click();
  await axis.getByRole('button', { name: /Release abc1234/ }).click();
  await expect(axis.getByRole('region', { name: 'Release abc1234 source record' })).toContainText('Install five-minute keepalive');
  await expect(axis.getByRole('region', { name: 'Release abc1234 source record' })).toContainText('release log line 12');
  await axis.getByRole('button', { name: /19:00 UTC: 1 hands/ }).click();
  await axis.getByRole('button', { name: /Five-minute keepalive enabled/ }).click();
  await expect(axis.getByRole('region', { name: 'Five-minute keepalive enabled source record' })).toContainText('ticket 0145');

  await axis.getByLabel('Jump to UTC date').fill('2026-09-25');
  await expect(axis.getByText(/-6(?:\.0)? chips\/hand/)).toBeVisible();
  await axis.getByRole('button', { name: /hand-1/ }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
});
