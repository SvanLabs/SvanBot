import { test, expect, type Page } from '@playwright/test';

// The public TV (`SVANBOT_TV_PORT`): the same table view on a listener with no operator token. The
// arena is the real one — `scripts/web-test-server.sh` binds a TV port beside the dashboard's, so
// `/api/health` here answers `{"public":true,…}` for real, unmocked, as the other specs leave it.
// What this test supplies is the hand a table view needs, in exactly the shape the projection sends
// it: the public payload drops `hole`, `decision`, `street`, `read` and the rest
// (`crates/apps/bot/src/api/tv.rs`), so a view that reads one of them has to say what it draws
// without it. An exception while rendering is the failure this file exists for.
const TV = `http://127.0.0.1:${process.env.SV10_TEST_TV_PORT || '8787'}`;

/** The projection, key for key: `PUBLIC_BOT_KEYS` and `PUBLIC_SEAT_KEYS`, and nothing else. */
function publicTable() {
  return {
    public: true,
    bots: [{
      slot: 0, name: 'TestBot', mode: 'playing', status: 'playing at table deadbeef', connected: true,
      table_id: 'deadbeef-0000', hand_id: 'h-public-tv', board: ['2c', '3d', '4s'],
      seats: [
        {seat: 0, name: 'TestBot', stack: 1800, bet: 0, folded: false, status: 'in hand', last_action: 'check', avatar_url: null},
        {seat: 1, name: 'Villain', stack: 950, bet: 1240, folded: false, status: 'in hand', last_action: 'raise', avatar_url: null},
      ],
      hero_seat: 0, dealer_seat: 1, actor_seat: 1, pot: 1240, big_blind: 20,
    }],
  };
}

async function quietStream(page: Page) {
  // The sandbox's own stream would replace the mocked table with the idle one; keep it open, silent.
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
}

test('the TV renders a live table with no session, no operator affordances and no exception', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(String(error)));
  let sessions = 0;
  await page.route('**/api/session', route => { sessions++; return route.fulfill({json: {ok: true}}); });
  await quietStream(page);
  await page.route('**/api/tv', route => route.fulfill({json: publicTable()}));
  await page.goto(`${TV}/`);

  const table = page.locator('.tv-mode .table-stage');
  await expect(table.locator('.poker-felt')).toBeVisible();
  await expect(table.locator('.pot b')).toHaveText('1,240');
  await expect(table.locator('.seat-1 .seat-name')).toHaveText('Villain');
  await expect(table.locator('.seat-1 .seat-action')).toHaveText('To act');
  // The hero's cards are face down: the public payload carries no `hole`, and a spectator should see
  // a back rather than a blank seat, a missing card — or an exception, which is what reading
  // `hole[0]` off the payload would be.
  await expect(table.locator('.seat-0 .seat-cards .playing-card.back')).toHaveCount(2);
  await expect(table.locator('.seat-0 .seat-name')).toContainText('TestBot');

  // No operator affordances. A seat plate is a label, not a scouting-report button; the commentary is
  // built from decision events this stream does not carry; and there is no dashboard to exit to.
  await expect(page.locator('.seat-plate.scoutable')).toHaveCount(0);
  await expect(page.locator('.tv-commentary')).toHaveCount(0);
  await expect(page.getByRole('link', {name: 'Exit TV'})).toHaveCount(0);
  // No login form, and the listener's only session route was never asked for: it has none.
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  expect(sessions, 'the TV asked for an operator session').toBe(0);
  expect(errors, 'the page raised while rendering the public payload').toEqual([]);
});
