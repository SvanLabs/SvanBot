import { test, expect, type Page } from '@playwright/test';

// 0305: a bet and the pot are drawn as stacks by denomination, a folded seat mucks its cards away
// and an all-in seat shoves. CSS transforms only, so the checks here are on classes and on which
// animation the element would run; the numbers beside the chips stay plain text.
const SHOT = process.env.SV10_SHOT_DIR || '/tmp/svw';

async function tableDashboard(page: Page) {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  // The sandbox's event stream would overwrite the mocked table; keep it open but quiet.
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    Object.assign(state.bots[0], {
      connected: true, mode: 'playing', status: 'playing at table 50ea6d9a', hand_id: 'h-0305',
      // The all-in seat is not the actor: a shoved player has nothing left to act on.
      hero_seat: 0, dealer_seat: 1, actor_seat: 0, street: 'flop', pot: 1240,
      board: ['2c', '3d', '4s'], hole: ['Ah', 'Kd'],
      seats: [
        {seat: 0, name: 'Test hero', stack: 1800, bet: 0, folded: false, last_action: 'check'},
        {seat: 1, name: 'Shover', stack: 0, bet: 1240, folded: false, last_action: 'all_in'},
        {seat: 2, name: 'Folder', stack: 950, bet: 0, folded: true, last_action: 'fold'},
      ],
    });
    await route.fulfill({json: state});
  });
}

test('chips read by denomination, the pot stacks, a fold mucks and an all-in shoves', async ({ page }) => {
  await tableDashboard(page);
  await page.goto('/');
  const table = page.locator('[data-widget="table"]');
  await expect(table.locator('.poker-felt')).toBeVisible();

  // 1240 = 1000 + 100 + 100 + 25 + 5 + 5, the six-chip cap; the colours carry the values.
  const bet = table.locator('.bet-spot .chip-stack').first();
  await expect(bet.locator('i.chip-1000')).toHaveCount(1);
  await expect(bet.locator('i.chip-100')).toHaveCount(2);
  await expect(bet.locator('i.chip-25')).toHaveCount(1);
  await expect(bet.locator('i.chip-5')).toHaveCount(2);
  await expect(bet.locator('i')).toHaveCount(6);
  await expect(bet).toHaveAttribute('aria-hidden', 'true');
  await expect(table.locator('.bet-spot b').first()).toHaveText('1,240');

  // The pot carries its own stack, drawn from the same denominations, and never replaces the number.
  const pot = table.locator('.pot');
  await expect(pot.locator('.pot-chips i.chip-1000')).toHaveCount(1);
  await expect(pot.locator('.pot-chips i')).toHaveCount(6);
  await expect(pot.locator('b')).toHaveText('1,240');

  // A folded seat keeps its cards in the DOM so they can be mucked away; the hero's never are.
  await expect(table.locator('.seat-2.seat-cards, .seat-2 .seat-cards')).toHaveCount(1);
  await expect(table.locator('.seat-2 .seat-cards')).toHaveCSS('animation-name', 'muck-away');
  await expect(table.locator('.seat-0 .seat-cards')).not.toHaveCSS('animation-name', 'muck-away');

  // The all-in seat shoves toward the middle, then keeps its glow.
  const shove = table.locator('.seat-1 .seat-plate');
  await expect(shove).toHaveCSS('animation-name', 'all-in-shove, all-in-glow');

  await page.screenshot({path: `${SHOT}/chips-0305.png`, fullPage: false});
  await table.locator('.table-stage').screenshot({path: `${SHOT}/chips-0305-stage.png`});
});

test('with reduced motion a fold shows no cards and nothing shoves', async ({ page }) => {
  await page.emulateMedia({reducedMotion: 'reduce'});
  await tableDashboard(page);
  await page.goto('/');
  const table = page.locator('[data-widget="table"]');
  await expect(table.locator('.poker-felt')).toBeVisible();
  // The muck is motion, so under reduced motion the folded seat simply shows nothing, as before.
  await expect(table.locator('.seat-2 .seat-cards')).toBeHidden();
  await expect(table.locator('.seat-1 .seat-plate')).toHaveCSS('animation-name', 'none');
  await expect(table.locator('.seat-0 .seat-cards .playing-card').first()).toBeVisible();
});

test('the chips stay inside a phone-width page', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await tableDashboard(page);
  await page.goto('/');
  const table = page.locator('[data-widget="table"]');
  await expect(table.locator('.poker-felt')).toBeVisible();
  // The pot stack hangs outside the pot box; it must not push the page sideways.
  await expect(table.locator('.pot-chips')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.screenshot({path: `${SHOT}/chips-0305-phone.png`});
});
