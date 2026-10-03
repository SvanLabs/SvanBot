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
  // Every `/api/` route the page asks for, in order. This listener serves its table and its stream;
  // any other one is a request the dashboard made, and the listener's answer to it is a 404.
  const asked: string[] = [];
  page.on('request', request => { const {pathname} = new URL(request.url()); if (pathname.startsWith('/api/')) asked.push(pathname); });
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
  // The board and its panels belong to the dashboard: the TV keeps its fixed default, and a stored
  // operator layout cannot reach it (#729).
  await expect(page.locator('.dashboard-grid, .panel-heading')).toHaveCount(0);
  // No login form, and the listener's only session route was never asked for: it has none.
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  expect(sessions, 'the TV asked for an operator session').toBe(0);
  expect(errors, 'the page raised while rendering the public payload').toEqual([]);
  // The shell drawn before `/health` answers is gone: it is a first paint, not a state.
  await expect(page.locator('.boot')).toHaveCount(0);
  // `/health` first — the page does not know which listener served it until that answers — then its
  // table, and nothing else. Mounting the dashboard before that answer instead is what made this
  // list six requests for panel routes this listener does not have, and made each one a 404
  // (issue #335). The order matters as much as the set: the panels are the page deciding what to
  // show before it knows where it is.
  expect(asked, 'the TV asked its listener for routes it does not serve').toEqual(['/api/health', '/api/tv']);
});

test('the TV keeps the base theme and the arena table, with no theme picker', async ({ page }) => {
  // The public TV is a spectator's screen, not the operator's: it has no settings modal, so it has
  // no theme picker, and it stays on the base look even when this origin's storage asks for light.
  await page.addInitScript(() => localStorage.setItem('svan-theme', 'light'));
  await quietStream(page);
  await page.route('**/api/tv', route => route.fulfill({json: publicTable()}));
  await page.goto(`${TV}/`);

  await expect(page.locator('.tv-mode .table-stage')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('group', {name:'Control room theme'})).toHaveCount(0);
  await expect(page.getByRole('button', {name:'Open settings'})).toHaveCount(0);
  await expect(page.locator('.tv-mode .table-theme-arena')).toHaveCount(1);
});
