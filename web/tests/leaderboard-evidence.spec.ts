import { expect, test } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

test('season race exposes throughput, active identities, and the evidenced opportunity', async ({page}) => {
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  await page.route('**/api/leaderboard', route => route.fulfill({json:{
    entries:[{rank:7,name:'SvanBotV7',score:841506,hands:1000,ours:true,
      gap_to_first:1211706,gap_to_next:200000,gap_to_four:500000,
      score_velocity_per_hour:1250,hands_velocity_per_hour:84.5}],
    season:{season_number:12,time_remaining_seconds:86400},updated:100,stale:false,error:null,
  }}));
  const now = Date.now() / 1000;
  await page.route('**/api/fleet', route => route.fulfill({json:{bots:[{
    name:'SvanBotV7',hands:1000,total:600,points:[
      {hand:900,ts:now - 7200,total:100},{hand:1000,ts:now,total:600},
    ],
  }]}}));
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    state.training.leaderboard_evidence = {
      strategy:'sv10-ev-9', neural:'stat fallback', range:`fitted@${Math.round(now - 7200)}`,
      calibration:`updated@${Math.round(now - 300)}`, opportunity:null,
      next_candidate:'river-call defense · held-out evaluation',
    };
    await route.fulfill({json:state});
  });
  await page.goto('/');

  const race = page.locator('[data-widget="season-race"]');
  await expect(race.getByText('CHIPS / HOUR', {exact:true})).toBeVisible();
  await expect(race.getByText('+250 / hr', {exact:true})).toBeVisible();
  await expect(race.getByText('HANDS / HOUR', {exact:true})).toBeVisible();
  await expect(race.getByText('84.5 / hr', {exact:true})).toBeVisible();
  await expect(race.getByText('sv10-ev-9', {exact:true})).toBeVisible();
  await expect(race.getByText('stat fallback', {exact:true})).toBeVisible();
  // Stamps read as ages (`fitted@<unix>` -> "fitted 2 h ago").
  await expect(race.getByText('fitted 2 h ago · updated 5 min ago', {exact:true})).toBeVisible();
  await expect(race.getByText('No validated opportunity', {exact:true})).toBeVisible();
  await expect(race.getByText(/next candidate.*river-call defense.*evidence pending/i)).toBeVisible();
});
