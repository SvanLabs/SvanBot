import { test, expect, type Locator, type Page } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

// The sandboxed backend runs one dry-run bot (slot 0). Tests that need a fleet expand /api/state
// to `size` bots (slots 0..size-1) built from the real snapshot, apply their own changes, and replace
// the live event stream with a quiet open one so the real single-bot state never overwrites it.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function fleet(page: Page, mutate?: (state: any) => void, size = 5) {
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let base: any;
  await page.route('**/api/state', async route => {
    if (!base) base = await (await route.fetch()).json();
    const state = structuredClone(base);
    const bot = state.bots[0];
    state.bots = Array.from({length: size}, (_, slot) => ({...structuredClone(bot), slot, name: slot === 0 ? bot.name : `TestBot${slot + 1}`}));
    mutate?.(state);
    await route.fulfill({json: state});
  });
}

async function box(locator: Locator) {
  await expect(locator).toBeVisible();
  const value = await locator.boundingBox();
  expect(value, 'visible element has a bounding box').not.toBeNull();
  expect([value!.x, value!.y, value!.width, value!.height].every(Number.isFinite)).toBe(true);
  expect(value!.width).toBeGreaterThan(0);
  expect(value!.height).toBeGreaterThan(0);
  return value!;
}

async function expectInside(child: Locator, parent: Locator, tolerance = 2) {
  const [inner, outer] = await Promise.all([box(child), box(parent)]);
  expect(inner.x).toBeGreaterThanOrEqual(outer.x - tolerance);
  expect(inner.y).toBeGreaterThanOrEqual(outer.y - tolerance);
  expect(inner.x + inner.width).toBeLessThanOrEqual(outer.x + outer.width + tolerance);
  expect(inner.y + inner.height).toBeLessThanOrEqual(outer.y + outer.height + tolerance);
}

async function mockLeaderboard(page: Page) {
  const names = ['AlphaBot', 'BetaBot', 'account2', 'account1', 'GammaBot', 'DeltaBot', 'SvanBotV7'];
  await page.route('**/api/leaderboard', route => route.fulfill({json:{
    entries:names.map((name, index) => ({
      rank:index + 1, name, score:2_000_000 - index * 120_000, hands:10_000,
      win_rate:1.2, ours:name === 'SvanBotV7', rank_delta:index % 3 - 1,
      gap_to_first:index * 120_000,
    })),
    season:{season_number:12,time_remaining_seconds:86400},
  }}));
}

test('alignment contract contains panel chrome, widgets, tables and season columns', async ({page}) => {
  await fleet(page, state => {
    for (const bot of state.bots) Object.assign(bot, {
      hand_id:'alignment', connected:true, mode:'playing', hero_seat:3, actor_seat:1,
      board:['9c','8h','3h','Ts','Ad'], hole:['As','Kd'], pot:123456,
      seats:Array.from({length:6}, (_, seat) => ({seat, name:`Opponent${seat}`, stack:123456, bet:1200, folded:false, last_action:'call'})),
    });
  });
  await mockLeaderboard(page);
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.goto('/');

  for (const viewport of [{width:1440,height:1000},{width:1024,height:900}]) {
    await page.setViewportSize(viewport);
    for (const theme of ['Arena','Felt','Midnight']) {
      await page.getByRole('button',{name:theme,exact:true}).click();
      await expectInside(page.locator('.table-panel .table-stage'), page.locator('.table-panel'));
    }
    const headings = page.locator('.panel-heading:visible');
    for (let index = 0; index < await headings.count(); index += 1) {
      const heading = headings.nth(index);
      await expectInside(heading, heading.locator('..'));
      const title = await box(heading.locator('h2'));
      const controls = await box(heading.locator('.panel-heading-actions'));
      const overlapX = Math.min(title.x + title.width, controls.x + controls.width) - Math.max(title.x, controls.x);
      const overlapY = Math.min(title.y + title.height, controls.y + controls.height) - Math.max(title.y, controls.y);
      expect(overlapX > 1 && overlapY > 1, `panel heading ${index} title and controls overlap`).toBe(false);
    }
    const widgets = page.locator('[data-widget]:visible');
    for (let index = 0; index < await widgets.count(); index += 1) {
      const widget = widgets.nth(index);
      await expectInside(widget, widget.locator('..'));
    }
    const rows = page.locator('.race-row');
    expect(await rows.count()).toBeGreaterThan(1);
    for (const selector of ['.race-rank','.race-name','.race-delta','.race-score','.race-bar']) {
      const lefts = await rows.locator(selector).evaluateAll(elements => elements.map(element => element.getBoundingClientRect().left));
      expect(Math.max(...lefts) - Math.min(...lefts), `${selector} column alignment`).toBeLessThanOrEqual(2);
    }
  }
});

test('Watch All geometry contains five themed tables without overlapping cards', async ({page}) => {
  await fleet(page, state => {
    for (const bot of state.bots) Object.assign(bot, {
      hand_id:'watch-all', connected:true, mode:'playing', hero_seat:3, actor_seat:1,
      board:['9c','8h','3h'], hole:['As','Kd'], pot:123456,
      seats:Array.from({length:6}, (_, seat) => ({seat, name:`Opponent${seat}`, stack:123456, bet:1200, folded:false, last_action:'call'})),
    });
  });
  await mockLeaderboard(page);
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.goto('/');
  await page.getByRole('button',{name:'Watch all',exact:true}).click();
  await expect(page.locator('.fleet-table')).toHaveCount(5);

  for (const viewport of [{width:1024,height:900},{width:1440,height:1000}]) {
    await page.setViewportSize(viewport);
    for (const theme of ['Arena','Felt','Midnight']) {
      await page.getByRole('button',{name:theme,exact:true}).click();
      const cards = page.locator('.fleet-table');
      for (let index = 0; index < 5; index += 1) {
        await expectInside(cards.nth(index).locator('.table-stage'), cards.nth(index));
      }
      const rectangles = await cards.evaluateAll(elements => elements.map(element => element.getBoundingClientRect().toJSON()));
      for (let a = 0; a < rectangles.length; a += 1) for (let b = a + 1; b < rectangles.length; b += 1) {
        const x = Math.min(rectangles[a].right, rectangles[b].right) - Math.max(rectangles[a].left, rectangles[b].left);
        const y = Math.min(rectangles[a].bottom, rectangles[b].bottom) - Math.max(rectangles[a].top, rectangles[b].top);
        expect(x > 1 && y > 1, `${theme} fleet cards ${a} and ${b} overlap`).toBe(false);
      }
    }
  }
});

test('season race geometry leads the default mobile operator priority without overflow', async ({page}) => {
  await fleet(page, state => Object.assign(state.bots[0], {connected:true,mode:'playing'}));
  await mockLeaderboard(page);
  await page.setViewportSize({width:390,height:844});
  await page.goto('/');
  await expect(page.locator('[data-widget="season-race"] .race-hero')).toBeVisible();
  await expectInside(page.locator('.table-panel .table-stage'), page.locator('.table-panel'));
  await expectInside(page.locator('.workspace-tools'), page.locator('.workspace-header'));
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);

  const priority = ['season-race','table','champion','opponents'];
  const tops = await Promise.all(priority.map(async id => (await box(page.locator(`[data-widget="${id}"]`))).y));
  expect(tops, 'season/rank, live decision, strategy/model, then deeper analysis').toEqual([...tops].sort((a,b) => a-b));
});

test('saved mobile widget order remains authoritative', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('svan-layout:v1', JSON.stringify({
    left:['autonomy','experiments','calibration','highlights','health','privacy'],
    center:['opponents','table','ranges','ticker','leaks','fleet-race','starting-hands','recent-hands'],
    right:['monitor','updates','season-race','performance','champion','season','activity'],
    hidden:[],
  })));
  await fleet(page);
  await mockLeaderboard(page);
  await page.setViewportSize({width:390,height:844});
  await page.goto('/');
  await expect(page.locator('.dashboard-grid')).toHaveClass(/custom-layout/);
  const opponents = await box(page.locator('[data-widget="opponents"]'));
  const table = await box(page.locator('[data-widget="table"]'));
  expect(opponents.y).toBeLessThan(table.y);
  await expectInside(page.locator('[data-widget="opponents"]'), page.locator('[data-widget="opponents"]').locator('..'));
  await expectInside(page.locator('.table-panel .table-stage'), page.locator('.table-panel'));
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
});

test('a table rearranged into a narrow side column remains contained', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('svan-layout:v1', JSON.stringify({
    left:['table','autonomy','experiments','calibration','highlights','health','privacy'],
    center:['ranges','ticker','leaks','fleet-race','starting-hands','recent-hands','opponents'],
    right:['monitor','updates','season-race','performance','champion','season','activity'],
    hidden:[],
  })));
  await fleet(page, state => Object.assign(state.bots[0], {
    connected:true,mode:'playing',hero_seat:3,actor_seat:1,board:['9c','8h','3h'],hole:['As','Kd'],
    seats:Array.from({length:6},(_,seat)=>({seat,name:`Opponent${seat}`,stack:2000,bet:20,folded:false,last_action:'call'})),
  }));
  await page.setViewportSize({width:1024,height:900});
  await page.goto('/');
  const widget = page.locator('[data-widget="table"]');
  const bounds = await box(widget);
  expect(bounds.width).toBeLessThan(300);
  await expectInside(widget.locator('.table-stage'), widget);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
});

test('occupied tables keep seats and bets clear at phone, tablet and desktop widths', async ({page}) => {
  await fleet(page, state => {
    for (const bot of state.bots) Object.assign(bot, {
      hand_id:'layout', connected:true, mode:'playing', hero_seat:3, actor_seat:1,
      board:['9c','8h','3h','Ts','Ad'], hole:['As','Kd'], pot:123456,
      // 0212: opponents carry the models' read; the hero seat never does.
      seats:Array.from({length:6}, (_, seat) => ({seat, name:`Opponent${seat}`, stack:123456, bet:1200, folded:false, last_action:'call',
        read: seat === 3 ? null : {style:'Tight-aggressive', hands:4200, vpip:0.28, pfr:0.16, three_bet:0.07, fold_vs_bet:[0.5,0.6,0.7], response_ratio:[0.9,1.3,1.1], fold_offset:-0.64}})),
    });
  });
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.goto('/');
  await expect(page.locator('.table-panel .seat')).toHaveCount(6);
  await expect(page.locator('.table-panel .seat-read')).toHaveCount(5);
  await expect(page.locator('.table-panel .seat-read').first()).toHaveText('Tight-aggressive · 28/16');
  await expect(page.locator('.table-panel .seat-read').first()).toHaveAttribute('title', /folds ×0\.90 · calls ×1\.30/);
  await expect(page.locator('.table-panel .seat-read').first()).toHaveAttribute('title', /folds to our bets less than the model prices \(logit -0\.64\)/);
  const checkGeometry = async () => {
    const problems = await page.evaluate(() => {
      const problems: string[] = [];
      const rect = (e: Element) => e.getBoundingClientRect();
      const overlaps = (a: DOMRect, b: DOMRect) => Math.min(a.right,b.right)-Math.max(a.left,b.left)>1 && Math.min(a.bottom,b.bottom)-Math.max(a.top,b.top)>1;
      for (const table of document.querySelectorAll('.table-stage')) {
        const bounds = rect(table);
        const seats = [...table.querySelectorAll('.seat-plate')];
        const cards = [...table.querySelectorAll('.playing-card')];
        for (const [i, seat] of seats.entries()) {
          const box = rect(seat);
          if (box.left<bounds.left-1 || box.right>bounds.right+1) problems.push('clipped seat');
          if (seats.slice(i+1).some(other => overlaps(box,rect(other)))) problems.push('overlapping seats');
        }
        for (const bet of table.querySelectorAll('.bet-spot.filled')) {
          if (cards.some(card => overlaps(rect(bet),rect(card)))) problems.push('bet covers card');
        }
      }
      for (const stats of document.querySelectorAll('.decision-stats')) {
        if (stats.scrollWidth>stats.clientWidth+1) problems.push('clipped decision statistics');
      }
      if (document.documentElement.scrollWidth>innerWidth) problems.push('page overflow');
      return problems;
    });
    expect(problems).toEqual([]);
  };
  for (const theme of ['Arena','Felt','Midnight']) {
    await page.getByRole('button',{name:theme,exact:true}).click();
    for (const width of [320,768,1440]) {
      await page.setViewportSize({width,height:1000});
      await checkGeometry();
    }
  }
  await page.getByRole('button',{name:'Arena',exact:true}).click();
  await page.getByRole('button',{name:'Watch all',exact:true}).click();
  await expect(page.locator('.fleet-table')).toHaveCount(5);
  await checkGeometry();
});

test('arena results settle on the winning seat without an acting pulse', async ({page}) => {
  await fleet(page, state => {
    Object.assign(state.bots[0], {
      hand_id:'arena-result', connected:true, mode:'playing', hero_seat:3, actor_seat:3,
      board:['9c','8h','3h'], hole:['As','Kd'], pot:130,
      seats:[{seat:3,name:'WinnerBot',stack:1800,bet:50,folded:false,last_action:'call'}],
    });
  }, 1);
  await page.clock.install();
  await page.goto('/');
  await page.getByRole('button',{name:'Arena',exact:true}).click();
  const seat = page.locator('.table-panel .seat.hero');
  await expect(seat.locator('.seat-action')).toHaveText('To act');
  await page.evaluate(() => window.dispatchEvent(new CustomEvent('sv-live', {detail:{
    type:'result', ts:Date.now()/1000, slot:0, hand_id:'arena-result', winners:['WinnerBot'], net:130,
  }})));
  await expect(seat.locator('.seat-action')).toHaveText('Winner');
  await page.clock.runFor(2000);
  await expect(seat.locator('.seat-action')).toHaveText('Winner');
  await expect(seat).not.toHaveClass(/\bacting\b/);
  await expect(seat.locator('.seat-plate')).toHaveCSS('border-top-color','rgb(233, 195, 107)');
  await page.screenshot({path:'../artifacts/arena-winner-settled.png',fullPage:true});
});

test('remote login retries with an operator token without persisting it', async ({ page }) => {
  await page.route('**/api/session', async route => {
    if (route.request().postDataJSON()?.token !== 'browser-test-token') {
      await route.fulfill({status:403,json:{detail:'Operator token required'}});
    } else {
      await route.continue();
    }
  });
  await page.goto('/');
  await expect(page.getByLabel('Operator token')).toBeVisible();
  await page.getByLabel('Operator token').fill('browser-test-token');
  await page.getByRole('button',{name:'Unlock control room'}).click();
  await expect(page.getByText('SYSTEM ONLINE')).toBeVisible();
  await expect(page.getByLabel('Operator token')).not.toBeVisible();
  expect(await page.evaluate(() => JSON.stringify({...localStorage}))).not.toContain('browser-test-token');
});

test('local dashboard loads without exposing credentials', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByText('SYSTEM ONLINE')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Live table' })).toBeVisible();
  await expect(page.getByText('Every hand tells a story')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Start', exact: true })).toBeEnabled();
  expect(errors).toEqual([]);
  await page.screenshot({path:'../artifacts/dashboard-desktop.png',fullPage:true});
});

test('layout settings persist and mobile has no horizontal overflow', async ({ page }) => {
  await page.setViewportSize({width:390,height:844});
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Live table' })).toBeVisible();
  await page.getByRole('button', {name:'Open settings'}).click();
  await page.getByRole('switch', {name:'Compact layout'}).click();
  await expect(page.getByRole('switch', {name:'Compact layout'})).toHaveAttribute('aria-checked','true');
  await page.getByRole('button', {name:'Close settings'}).click();
  await page.reload();
  await expect(page.locator('.app')).toHaveClass(/compact/);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(overflow).toBe(false);
  await page.screenshot({path:'../artifacts/dashboard-mobile.png',fullPage:true});
});

test('start button sends the selected bot command', async ({ page }) => {
  let command: unknown;
  await page.route('**/api/bots/*/command', async route => {
    command = route.request().postDataJSON();
    await route.fulfill({json:{ok:true}});
  });
  await page.goto('/');
  await expect(page.getByText('SYSTEM ONLINE')).toBeVisible();
  await page.getByRole('button',{name:'Start',exact:true}).click();
  await expect.poll(()=>command).toEqual({command:'start'});
});

test('pause and stop remain available for an active bot', async ({ page }) => {
  const commands: string[] = [];
  await fleet(page, state => Object.assign(state.bots[0], {mode:'playing', connected:true}), 1);
  await page.route('**/api/bots/0/command', async route => {
    commands.push(route.request().postDataJSON().command);
    await route.fulfill({json:{ok:true}});
  });
  await page.goto('/');
  await page.getByRole('button', {name:'Pause', exact:true}).click();
  await page.getByRole('button', {name:'Stop', exact:true}).click();
  await expect.poll(() => commands).toEqual(['pause', 'stop']);
});

test('an interrupted live stream keeps stale state visible and disables controls', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.goto('/');
  await expect(page.getByText('Connection interrupted.', {exact:false})).toBeVisible();
  await expect(page.getByRole('heading', {name:'Live table'})).toBeVisible();
  await expect(page.getByRole('button', {name:'Start', exact:true})).toBeDisabled();
});

test('leaderboard keeps the last snapshot visible and labels it stale after refresh failure', async ({page}) => {
  await page.clock.install();
  let requests = 0;
  await page.route('**/api/leaderboard', async route => {
    requests += 1;
    if (requests === 1) {
      await route.fulfill({json:{entries:[{
        rank:7, name:'SvanBotV7', score:841506, hands:1000, win_rate:1.2,
        ours:true, rank_delta:null, gap_to_first:1211706, gap_to_next:200000,
        gap_to_four:500000, score_delta:null, score_velocity_per_hour:null,
      }], season:{season_number:12,time_remaining_seconds:86400},updated:100,stale:false,error:null}});
    } else {
      await route.fulfill({status:503,json:{detail:'leaderboard unavailable'}});
    }
  });
  await page.goto('/');
  await expect(page.getByText('SvanBotV7 is #7')).toBeVisible();
  await expect(page.locator('.race-score')).toHaveText('841,506 pts');
  await expect(page.locator('.race-score')).toHaveAttribute('title', /not poker profit/i);
  await page.clock.runFor(60_100);
  await expect.poll(() => requests).toBeGreaterThan(1);
  await expect(page.getByText('SvanBotV7 is #7')).toBeVisible();
  await expect(page.getByText(/leaderboard data stale/i)).toBeVisible();
  await expect(page.locator('[data-widget="season-race"]').getByText('LIVE', {exact:true})).toHaveCount(0);
});

test('initial leaderboard failure renders rank and gaps as unavailable', async ({page}) => {
  await page.route('**/api/leaderboard', route => route.fulfill({status:503,json:{detail:'leaderboard unavailable'}}));
  await page.goto('/');
  await expect(page.getByText(/rank unavailable/i)).toBeVisible();
  await expect(page.getByText(/gap to #1 unavailable/i)).toBeVisible();
  await expect(page.getByText(/#0|0 chips behind #1/i)).toHaveCount(0);
});

test('Season race shows loading, every target and velocity uncertainty on phone', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await page.route('**/api/leaderboard', async route => {
    await new Promise(resolve => setTimeout(resolve, 250));
    await route.fulfill({json:{entries:[{
      rank:7,name:'SvanBotV7',score:841506,hands:1000,win_rate:1.2,pro:false,ours:true,
      rank_delta:null,gap_to_first:1211706,gap_to_next:200000,gap_to_four:500000,
      score_delta:500,score_velocity_per_hour:1250,
    }],season:{season_number:12,time_remaining_seconds:86400},updated:100,stale:false,error:null}});
  });
  await page.goto('/');
  await expect(page.getByText('Loading leaderboard', {exact:true}).first()).toBeVisible();
  await expect(page.getByText('SvanBotV7 is #7')).toBeVisible();
  const race = page.locator('[data-widget="season-race"]');
  await expect(race.getByText('GAP TO #1', {exact:true})).toBeVisible();
  await expect(race.getByText('1,211,706', {exact:true})).toBeVisible();
  await expect(race.getByText('GAP TO #4', {exact:true})).toBeVisible();
  await expect(race.getByText('500,000', {exact:true})).toBeVisible();
  await expect(race.getByText('GAP TO NEXT', {exact:true})).toBeVisible();
  await expect(race.getByText('200,000', {exact:true})).toBeVisible();
  await expect(race.getByText('RECENT ESTIMATE', {exact:true})).toBeVisible();
  await expect(race.getByText('+1,250 / hr', {exact:true})).toBeVisible();
  await expect(race.getByText(/short-window estimate.*variance/i)).toBeVisible();
  const priority = ['season-race','table','monitor','performance','champion','leaks','opponents'];
  const tops = await Promise.all(priority.map(async id => (await page.locator(`[data-widget="${id}"]`).boundingBox())!.y));
  expect(tops).toEqual([...tops].sort((a,b) => a-b));
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
});

test('decision telemetry reports native sampling error, exact computation, and missing evidence', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await fleet(page, state => {
    Object.assign(state.bots[0], {version: 'sv10-ev-1', decision: {action: 'check', amount: 0, reason: 'No bet to call', source: 'range-equity',
      latency_ms: 0, version: 'sv10-ev-1', equity: {value: .625, samples: 4000, standard_error: .0125, exact: false}}});
    Object.assign(state.bots[1], {version: 'sv10-ev-1', decision: {action: 'call', amount: 20, reason: 'Exact river model', source: 'range-equity',
      latency_ms: 9, version: 'sv10-ev-1', equity: {value: .625, samples: 1081, standard_error: 0, exact: true}}});
    Object.assign(state.bots[2], {decision: null});
  });
  await page.goto('/');
  const telemetry = page.getByRole('region', {name:'Decision telemetry'});
  await expect(telemetry.getByText('0 ms', {exact:true})).toBeVisible();
  await expect(telemetry.getByText('SAMPLING ERROR', {exact:true})).toBeVisible();
  await expect(telemetry.getByText('1.25 pp SE', {exact:true})).toBeVisible();
  await expect(telemetry.getByText('Monte Carlo · 4,000 samples', {exact:true})).toBeVisible();
  await expect(telemetry.getByText('Range equity', {exact:true})).toBeVisible();
  await page.screenshot({path:'../artifacts/dashboard-telemetry.png',fullPage:true});
  await page.locator('.bot-tabs button').nth(1).click();
  await expect(telemetry.getByText('EQUITY COMPUTATION', {exact:true})).toBeVisible();
  await expect(telemetry.getByText('Exact for modeled range', {exact:true})).toBeVisible();
  await page.locator('.bot-tabs button').nth(2).click();
  await expect(telemetry.getByText('Unavailable', {exact:true})).toHaveCount(3);
});

// 0299: the signal strip carries the whole decision record — the plan in words, every priced option
// with the move taken marked, the price gauge, the fold chance, the arm and the analyst's grade.
const signalDecision = {
  action: 'raise', amount: 90, source: 'ev-policy', version: 'sv10-ev-43', latency_ms: 367.565162,
  street: 'turn', hand_category: 'Pair', pot: 322, to_call: 90, pot_odds: 0.2194,
  reason: 'eq 0.27 vs 1 opp, raise ev 40 (best 74), they fold 2% — check  ev 74 · raise 90 ev 40 · raise 151 ev 6 · raise 219 ev -24 · raise 328 ev -90',
  equity: {value: 0.273, samples: 1_173_926, standard_error: 0.0004, exact: false},
  opponent_models: [{name: 'EpsilonBot'}],
  candidates: [
    {action: 'check', amount: null, ev: 74, equity_called: 0.2727, fold_prob: 0},
    {action: 'raise', amount: 90, ev: 40, equity_called: 0.2585, fold_prob: 0.0199},
    {action: 'raise', amount: 151, ev: 5.6, equity_called: 0.2288, fold_prob: 0.0592},
    {action: 'raise', amount: 219, ev: -24, equity_called: 0.2139, fold_prob: 0.0771},
    {action: 'raise', amount: 328, ev: -90, equity_called: 0.1948, fold_prob: 0.0993},
  ],
};

test('the signal strip reads the decision, prices every option and marks the one it took', async ({ page }) => {
  await fleet(page, state => {
    Object.assign(state.bots[0], {name: 'SvanBotV10', hand_id: 'h-signal', decision: signalDecision});
  });
  await page.route('**/api/accuracy', route => route.fulfill({json: {days: 7, fleet: {decisions: 1, accuracy: 99, grades: [1,0,0,0,0], mean_loss_bb: 0}, bots: [], streets: [], worst: [
    {ts: 1790460517, bot: 'SvanBotV10', hand_id: 'h-signal', street: 'turn', live_action: 'raise:90', deep_action: 'call', loss_bb: 14.8, pot_bb: 67.4, grade: 'mistake'},
  ]}}));
  await page.route('**/api/experiment', route => route.fulfill({json: {status: 'running', season: 's', qualifying: {readings: 3, needed: 3}, protected: [], pair: ['SvanBotV10', 'TestBot2'], ranks: {}, last_reading_at: null, last_attempt_at: null, reading_age_secs: 10, stale_after_secs: 480, last_error: null, last_transition: null, reason: null,
    bots: [{name: 'SvanBotV10', role: 'treatment', hands_on_target: 24, last_assignment_change: null}], target: null, verdicts: []}}));
  await page.goto('/');
  const rail = page.getByRole('region', {name:'Decision telemetry'});
  // The plan in words: street, hand, equity against the field, the price, the move, the fold chance.
  await expect(rail.getByText('Turn, pair: equity 27% against 1 opponent; a 90-chip call needs 22% and this hand has 27%; the policy raises to 90 — +40 chips, 34 behind the best option (Check at +74); that bet folds them 2% of the time.', {exact:true})).toBeVisible();
  // Latency against the clock the server auto-acts on.
  await expect(rail.getByText('368 ms', {exact:true})).toBeVisible();
  await expect(rail.getByText('0.8% of the 45 s turn deadline · 44.6 s to spare', {exact:true})).toBeVisible();
  // Every option it priced, with the move it took marked and the fold chance beside each bet size.
  const rows = rail.locator('.ev-row');
  await expect(rows).toHaveCount(5);
  await expect(rows.nth(0)).toContainText('Check');
  await expect(rows.nth(1)).toContainText('Raise 90');
  await expect(rows.nth(1)).toContainText('CHOSEN');
  await expect(rows.nth(1)).toContainText('+40');
  await expect(rows.nth(4)).toContainText('Raise 328');
  await expect(rows.nth(4)).toContainText('-90');
  await expect(rows.nth(4)).toContainText('9.9%');
  await expect(rail.locator('.ev-row.chosen')).toHaveCount(1);
  await expect(rail.locator('.ev-row.best')).toHaveCount(1);
  // The gauge puts equity against the price, and the facts name the fold chance, the arm and the grade.
  await expect(rail.getByRole('img', {name:'Equity 27% against a call price of 22%'})).toBeVisible();
  await expect(rail.getByText('A 90-chip call needs 22% of the pot and this hand has 27% — above the price.')).toBeVisible();
  await expect(rail.getByText('THEIR FOLD CHANCE')).toBeVisible();
  await expect(rail.getByText('Raise 90 — the move it took · the chance every opponent still in the hand folds, from the same response model the EV uses')).toBeVisible();
  await expect(rail.getByText('treatment', {exact:true})).toBeVisible();
  await expect(rail.getByText(/Live now · 24 hands on target · pair SvanBotV10 vs TestBot2/)).toBeVisible();
  await expect(rail.getByText('DEEP RE-SOLVE')).toBeVisible();
  await expect(rail.locator('.gr-chip')).toHaveText('Mistake');
  await expect(rail.getByText('call was best')).toBeVisible();
  await expect(rail.getByText(/live raise 90 gave up 14.8 bb of a 67 bb pot · turn · graded/)).toBeVisible();
  // The table's own strip keeps the decision detail and now reads as the same sentence, with the
  // record's shorthand in the tooltip.
  const strip = page.locator('.decision-strip p').first();
  await expect(strip).toContainText('the policy raises to 90');
  await expect(strip).toHaveAttribute('title', /eq 0\.27 vs 1 opp, raise ev 40 \(best 74\)/);
});

test('the signal strip says only what the record backs', async ({ page }) => {
  await fleet(page, state => {
    Object.assign(state.bots[0], {name: 'TestBot', hand_id: 'h-other', decision: {
      action: 'call', amount: 90, source: 'ev-policy', latency_ms: 9, street: 'river', hand_category: 'Pair', pot: 322, to_call: 90, pot_odds: 0.2194,
      reason: 'eq 0.31 vs 2 opp, call ev 12 (best 12)', equity: {value: 0.31, samples: 4000, standard_error: 0.006, exact: false}}});
  });
  // A grade for another hand of the same bot, and no live experiment: neither may appear.
  await page.route('**/api/accuracy', route => route.fulfill({json: {days: 7, fleet: {decisions: 1, accuracy: 99, grades: [1,0,0,0,0], mean_loss_bb: 0}, bots: [], streets: [], worst: [
    {ts: 1790460517, bot: 'TestBot', hand_id: 'h-somewhere-else', street: 'river', live_action: 'call:90', deep_action: 'fold', loss_bb: 9, pot_bb: 30, grade: 'blunder'},
  ]}}));
  await page.route('**/api/experiment', route => route.fulfill({json: {status: 'champion', season: 's', qualifying: {readings: 0, needed: 3}, protected: [], pair: [], ranks: {}, last_reading_at: null, last_attempt_at: null, reading_age_secs: null, stale_after_secs: 480, last_error: null, last_transition: null, reason: null, bots: [], target: null, verdicts: []}}));
  await page.goto('/');
  const rail = page.getByRole('region', {name:'Decision telemetry'});
  // No option table in this record: no chart, and the policy's own words survive in the sentence.
  await expect(rail.locator('.ev-bars')).toHaveCount(0);
  await expect(rail.getByText(/the policy calls — eq 0\.31 vs 2 opp, call ev 12 \(best 12\)/)).toBeVisible();
  await expect(rail.getByText('DEEP RE-SOLVE')).toHaveCount(0);
  await expect(rail.getByText('Champion everywhere', {exact:true})).toBeVisible();
  await expect(rail.getByText('No experiment arm is live for this bot')).toBeVisible();
  await expect(rail.getByText('Nothing to call', {exact:false})).toHaveCount(0);
  await expect(page.locator('.decision-strip p').first()).toContainText('the policy calls — eq 0.31 vs 2 opp');
});

test('fleet keyboard selection focuses a table and preserves controls', async ({ page }) => {
  await fleet(page);
  await page.goto('/');
  await page.getByRole('button', {name:'Watch all',exact:true}).click();
  const secondBot = page.locator('.fleet-heading').nth(1);
  await secondBot.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.fleet-table')).toHaveCount(0);
  await expect(page.getByRole('button', {name:'Start',exact:true})).toBeVisible();
  await expect(page.locator('.bot-tabs button').nth(1)).toHaveClass(/selected/);
});

test('hand replay reveals community cards by street', async ({ page }) => {
  await page.route('**/api/bots/0/hands', route => route.fulfill({json:[{id:1,hand_id:'test-hand',ts:1789041600,hole:['Ah','Kd'],board:['2h','3s','4d'],net:40,big_blind:20,version:'sv10-ev-1'}]}));
  await page.route('**/api/bots/0/hands/test-hand', route => route.fulfill({json:[{type:'hand_start',ts:1789041600,data:{}},{type:'community_cards',ts:1789041602,data:{cards:['2h','3s','4d']}},{type:'hand_result',ts:1789041605,data:{}}]}));
  await page.goto('/');
  await page.getByText('#test-hand',{exact:true}).click();
  const dialog = page.getByRole('dialog',{name:'Hand replay'});
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel('Ah',{exact:true})).toBeVisible();
  await dialog.getByLabel('Replay position').fill('1');
  await expect(dialog.getByLabel('2h',{exact:true})).toBeVisible();
  await dialog.getByRole('button',{name:'Close replay'}).click();
  await expect(dialog).not.toBeVisible();
});

test('watch all renders a live board for every configured bot', async ({ page }) => {
  await fleet(page);
  await page.goto('/');
  await expect(page.getByText('SYSTEM ONLINE')).toBeVisible();
  await page.getByRole('button', {name:'Watch all',exact:true}).click();
  await expect(page.locator('.fleet-table')).toHaveCount(5);
  await expect(page.locator('.fleet-table .board-cards')).toHaveCount(5);
  await expect(page.locator('.range-grid > span')).toHaveCount(169);
  await page.screenshot({path:'../artifacts/fleet-upgraded.png',fullPage:true});
  await page.locator('.fleet-heading').first().click();
  await expect(page.locator('.fleet-table')).toHaveCount(0);
});

test('known cards remain readable in folded seats and bets show chip stacks', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const response = await route.fetch();
    const state = await response.json();
    Object.assign(state.bots[0], {hole:['Ah','Kd'],board:['2c','3d','4s'],hero_seat:0,dealer_seat:1,actor_seat:1,pot:120,street:'flop',seats:[{seat:0,name:'Test hero',stack:1000,bet:40,folded:true},{seat:1,name:'Test opponent',stack:900,bet:80}]});
    await route.fulfill({json:state});
  });
  await page.goto('/');
  const card = page.locator('.seat.hero').getByLabel('Ah',{exact:true});
  await expect(card).toBeVisible();
  await expect(card).toHaveCSS('opacity','1');
  await expect(page.locator('.seat.hero')).toHaveCSS('opacity','1');
  await expect(page.locator('.bet-spot .chip-stack')).toHaveCount(2);
  await page.screenshot({path:'../artifacts/cards-and-chips.png',fullPage:true});
});

test('panels minimize to a bar and remember their state', async ({ page }) => {
  await page.goto('/');
  const panel = page.locator('section.panel').filter({has: page.getByRole('heading', {name: 'Starting hand library', exact: true})});
  await expect(panel.locator('.range-grid')).toBeVisible();
  await panel.getByRole('button', {name: 'Minimize Starting hand library'}).click();
  await expect(panel.locator('.range-grid')).toBeHidden();
  expect(await panel.evaluate(element => element.getBoundingClientRect().height)).toBeLessThan(65);
  await page.reload();
  await expect(panel.getByRole('button', {name: 'Expand Starting hand library'})).toHaveAttribute('aria-expanded', 'false');
  await panel.getByRole('button', {name: 'Expand Starting hand library'}).click();
  await expect(panel.locator('.range-grid')).toBeVisible();
});

test('automatic training does not ask the operator to run a cycle', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Automatic improvement enabled')).toBeVisible();
  await expect(page.getByRole('button', {name: 'Run training cycle', exact: true})).toHaveCount(0);
});

test('autonomy explains resumable population-aware experiment progress', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const response = await route.fetch();
    const state = await response.json();
    Object.assign(state.training, {
      status:'evaluating', phase:'evaluating', stratum:'observed', resumed:true,
      progress:{hands:12500,target:20000,challenger_decisions:1000,challenger_policy_hits:250},
      lineage:['baseline-v9','candidate-winner'],
      experiments:[{id:'adaptive',status:'evaluating',ts:1,candidate_kind:'parameters',knob:'open_width',old:1,new:1.05,
        hands:12500,target:20000,rationale:'Cover an untested bounded parameter direction',resumed:true,
        population:{id:'population-42',opponent_count:18,evidence:4200},
        strata:{synthetic:{hands:10000,lower_95:-.01,upper_95:.04},observed:{hands:2500,lower_95:.02,upper_95:.08}}}]
    });
    await route.fulfill({json:state});
  });
  await page.goto('/');
  await expect(page.getByText('Clone pool of the live opponents · resumed')).toBeVisible();
  await expect(page.getByText('18 opponents · 4,200 observations')).toBeVisible();
  await expect(page.getByText('Cover an untested bounded parameter direction')).toBeVisible();
  await expect(page.getByText('Synthetic 10,000')).toBeVisible();
  await expect(page.getByText('Clone pool 2,500 hands')).toBeVisible();
  await expect(page.getByText('25% policy coverage')).toBeVisible();
  await expect(page.getByText('baseline-v9 → candidate-winner')).toBeVisible();
});

test('an idle learner says what it waits for and lists what learned recently, without fake strata', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    const now = Date.now() / 1000;
    Object.assign(state.training, {
      status:'idle', automatic:true, phase:'waiting for new hands', progress:{hands:139,target:500},
      next_job:{kind:'refit',label:'Evidence refresh',hands:139,target:500,remaining:361,reason:'139 of 500 new hands for refreshed models and fits',season_day:6},
      learning:[{what:'Opponent models', updated: now - 60, detail:'265 players, 539416 observed hands; updated after every hand'},
                {what:'Neural response model', updated: now - 3 * 3600, detail:'in use · validation log-loss 0.6687 vs 0.7583 for the stat model · 645305 training samples'}],
      experiments:[{id:'x',status:'rejected',ts:1,knob:'open_bb',old:2.5,new:2.375,hands:13056,
        strata:{observed:{hands:13056,lower_95:-.079,upper_95:.001}}}]
    });
    await route.fulfill({json:state});
  });
  await page.goto('/');
  await expect(page.getByText('NEXT JOB · EVIDENCE REFRESH')).toBeVisible();
  await expect(page.getByText(/361 hands left.*139 of 500 new hands.*Season day 6/)).toBeVisible();
  await expect(page.getByText('Opponent models')).toBeVisible();
  await expect(page.getByText('1 min ago', {exact:true})).toBeVisible();
  await expect(page.getByText(/validation log-loss 0.6687/)).toBeVisible();
  await expect(page.getByText('Clone pool 13,056 hands')).toBeVisible();
  await expect(page.getByText(/^Synthetic/)).toHaveCount(0);
  // An idle learner has no candidate in flight; the next job panel says what it waits for instead.
  await expect(page.getByText('Next: parameter search', {exact:false})).toHaveCount(0);
});

test('a cooling-down search shows the clock, not an empty hand fraction', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    const now = Date.now() / 1000;
    Object.assign(state.training, {
      status:'idle', automatic:true, phase:'cooling down', progress:{hands:0,target:0},
      cooldown_until: now + 1620, cooldown_minutes: 60,
      next_run: now + 1620,
      next_job:{kind:'search',label:'Champion search',hands:0,target:0,remaining:0,
        reason:'champion search cooling down for 27 more min during season days 1-3', season_day:2},
    });
    await route.fulfill({json:state});
  });
  await page.goto('/');
  await expect(page.getByText('NEXT JOB · CHAMPION SEARCH')).toBeVisible();
  await expect(page.getByText(/cooling down for 27 more min during season days 1-3/)).toBeVisible();
  await expect(page.getByText(/Season day 2/)).toBeVisible();
  await expect(page.getByText(/Next automatic search:/)).toBeVisible();
  // No hand fraction and no bar: the search is waiting on the clock, not on evidence.
  await expect(page.locator('.training-progress b')).toHaveCount(0);
  await expect(page.locator('.training-progress .progress-track')).toHaveCount(0);
  await expect(page.getByRole('button', {name:'Start next search early'})).toBeVisible();
});

test('the Autonomy panel shows what the fleet found about its own play, with its evidence', async ({page}) => {
  await fleet(page, state => {
    state.training.findings = {at: 1_700_000_000, findings: [
      {id:'decision-loss:turn:raise', title:'turn raise costs 0.057 bb per decision', severity:'P0',
       evidence:'6,643 deep re-solves, 376.2 bb given up', value:0.057, since:1_699_000_000, updated:1_700_000_000, ticket:'0281-turn-betting'},
      {id:'calibration:flop:bet:big', title:'flop:bet:big realizes +2.4 bb over its uncorrected price', severity:'P2',
       evidence:'826 decisions over every pot size - a measurement, not a loss', value:2.4, since:1_699_500_000, updated:1_700_000_000}],
      cleared: [], unanswered: ['decision loss: no analyst re-solves']};
  });
  await page.goto('/');
  const panel = page.getByLabel('What the fleet found about its own play');
  await expect(panel.getByText('P0')).toBeVisible();
  await expect(panel.getByText('turn raise costs 0.057 bb per decision')).toBeVisible();
  await expect(panel.getByText(/6,643 deep re-solves, 376.2 bb given up . filed as 0281-turn-betting/)).toBeVisible();
  // A measurement is labelled as one, and a question that could not be answered is named.
  await expect(panel.getByText('P2')).toBeVisible();
  await expect(panel.getByText(/a measurement, not a loss/)).toBeVisible();
  await expect(panel.getByText(/Could not measure: decision loss/)).toBeVisible();
});

test('the Autonomy panel names stale loops instead of hiding them behind a healthy summary', async ({page}) => {
  await fleet(page, state => {
    state.training.stale_loops = [
      {name:'analyst', message:'analyst last reported 2.5 h ago (limit 1.0 h): big decisions are not being re-solved or graded'},
      {name:'backups', message:'backups has never reported: no hourly backup or integrity check has completed'},
    ];
  });
  await page.goto('/');
  await expect(page.getByText(/Stale loops — learning is degraded/)).toBeVisible();
  await expect(page.getByText(/analyst last reported 2\.5 h ago/)).toBeVisible();
  await expect(page.getByText(/backups has never reported/)).toBeVisible();
});

test('the operator sets the learner cooldown from the Autonomy panel and the server keeps it', async ({ page }) => {
  await page.route('**/api/events', route => route.abort());
  await page.goto('/');
  // One "Learner pacing" form holds the new-hands limit and the cooldown (0185).
  const box = page.getByRole('form', {name: 'Learner pacing'});
  const input = box.getByLabel(/cooldown between searches/i);
  const hands = box.getByLabel(/new hands before an evidence refresh/i);
  await expect(input).toHaveValue('60');
  await input.fill('5000');
  await expect(box.getByRole('button', {name: 'Save'})).toBeDisabled();
  await input.fill('45');
  await hands.fill('50000');
  await expect(box.getByRole('button', {name: 'Save'})).toBeDisabled();
  await hands.fill('300');
  await box.getByRole('button', {name: 'Save'}).click();
  await expect(box.getByText('Saved: refresh after 300 new hands; late-season search uses the same limit; 45 min search cooldown')).toBeVisible();
  await page.reload();
  await expect(page.getByRole('form', {name: 'Learner pacing'}).getByLabel(/cooldown between searches/i)).toHaveValue('45');
  await expect(page.getByRole('form', {name: 'Learner pacing'}).getByLabel(/new hands before an evidence refresh/i)).toHaveValue('300');
  const bad = await page.request.post('/api/training/settings', {data: {cooldown_minutes: -1}});
  expect(bad.status()).toBe(400);
});

test('panel information works while minimized and links to the help page', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', {name: 'Minimize Champion profile', exact:true}).click();
  await page.getByRole('button', {name: 'About Champion profile', exact:true}).click();
  const explanation = page.getByRole('region', {name:'Champion profile explanation'});
  await expect(explanation).toContainText('promotes only after fresh-deal confirmation');
  await explanation.getByRole('link', {name:'How the bot works'}).click();
  await expect(page.getByRole('heading', {name:'What is the bot doing?'})).toBeVisible();
  await page.getByRole('link', {name:'Thinking and waiting'}).click();
  await expect(page.getByRole('heading', {name:'What “Thinking” means'})).toBeVisible();
  await expect(page.getByText('It is not a live CPU indicator.', {exact:false})).toBeVisible();
  await page.reload();
  await expect(page.getByRole('heading', {name:'What “Thinking” means'})).toBeVisible();
  await page.getByRole('link', {name:'Back to control room'}).click();
  await expect(page.getByRole('button', {name:'Expand Champion profile', exact:true})).toBeVisible();
});

test('champion explanations and mobile help are readable', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByText('Opponent profiles tracked', {exact:true})).toBeVisible();
  await expect(page.getByText('Version for this hand', {exact:true})).toBeVisible();
  await expect(page.getByText('Minimum EV a call must show over folding', {exact:false})).toBeVisible();
  await expect(page.getByText('from their own re-raise rates', {exact:false})).toBeVisible();
  const panels = page.locator('section.panel');
  expect(await page.getByRole('button', {name:/^About /}).count()).toBe(await panels.count());
  await page.setViewportSize({width:390,height:844});
  await page.goto('/#help');
  await expect(page.getByRole('heading', {name:'What is the bot doing?'})).toBeVisible();
  await expect(page.getByText('profile clones', {exact:false}).first()).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.screenshot({path:'../artifacts/bot-help-mobile.png',fullPage:true});
});

const scoutRates = {vpip:0.3, pfr:0.2, open_raise:0.2, limp:0.05, three_bet:0.09, call_open:0.2, fold_to_3bet:0.5, four_bet:0.05, fold_to_4bet:0.4, cbet:0.7,
  fold_to_cbet:0.5, wtsd:0.3, won_showdown:0.52, river_bluff:0.25, bet_first:[0.4,0.3,0.3], fold_vs_bet:[0.4,0.5,0.6], raise_vs_bet:[0.1,0.08,0.05], vpip_pos:[0.2,0.35,0.3], open_pos:[0.15,0.3,0.2]};

test('an opponent row opens the one scout view with our record, the corrections and key hands', async ({ page }) => {
  // 0296: the Opponent intelligence table became a list, and the row opens the single view that
  // replaced the table, the read list and the profile drawer.
  await fleet(page, state => { state.bots[0].name = 'SvanBotV10'; });
  await page.route('**/api/bots/0/opponents', async route => {
    await route.fulfill({json: [{name:'Villain', style:'Loose / aggressive', evidence_hands:42,
      vpip:{value:.48,samples:420,count:200,lower:.4,upper:.56}, pfr:{value:.31,samples:420}, aggression:{value:.62,samples:300}, fold_to_bet:{value:.27,samples:310}}]});
  });
  await page.route('**/api/bots/0/hands/*', route => route.fulfill({json:[]}));
  await page.route('**/api/players/Villain/card', route => route.fulfill({json:{
    name:'Villain', avatar_url:null, style:'Loose / aggressive', advice:'They give up when the board misses.', hands_observed:42, confidence:0.62,
    read:{...scoutRates, vpip:0.48}, league:scoutRates, corrections:{fold_offset:-0.64, response_ratio:null, size_tell:0.34},
    leaderboard:{rank:12, score:98120, hands:4300, win_rate:0.61, rank_delta:2},
    reputation:{best_rank:4, seasons:7, top10:3, lifetime_hands:219605, strength:0.91, names:['Villain','villain_two'],
      finishes:[{season:10, rank:4, score:120000, hands:3000, participants:160}]},
    vs_us:{hands:42, net:900, ev_net:1200, bb100:107, confidence:null, ev_bb100:142, ev_confidence:null, won_pots:9, lost_pots:4,
      biggest_win:{hand_id:'h-1aaaaaaa', bot:'SvanBotV10', net:30000, pot:61000, ts:1790328893},
      biggest_loss:{hand_id:'h-2bbbbbbb', bot:'SurSvan', net:-1050, pot:2100, ts:1789626632},
      recent:[{hand_id:'h-1aaaaaaa', bot:'SvanBotV10', net:30000, pot:61000, ts:1790328893},
        {hand_id:'h-2bbbbbbb', bot:'SurSvan', net:-1050, pot:2100, ts:1789626632}],
      by_bot:[{bot:'SvanBotV10', hands:40, net:500}], form:['W','L','W'], series:[{hand:1,net:0,ev:0},{hand:42,net:900,ev:1200}]},
    vs_seat:{hands:42, bb_per_100:12.5, low_95:-30.2, high_95:55.1, beats_us:false, we_beat:false},
  }}));
  await page.goto('/');
  const row = page.getByRole('button', {name:/Villain/});
  await expect(row).toContainText('hands observed');
  await expect(row).toContainText('42');       // the evidence hand base, not VPIP opportunities
  await row.click();
  const card = page.getByRole('dialog', {name:'Scout view: Villain'});
  await expect(card).toBeVisible();
  // One hand base, stated where the numbers are read.
  await expect(card.getByText(/Read from 42 hands observed/)).toBeVisible();
  // Reputation and the best season rank come from the same payload.
  await expect(card.getByText('BEST SEASON RANK')).toBeVisible();
  await expect(card.getByText('#4', {exact:true})).toBeVisible();
  await expect(card.getByText(/Also played as/)).toContainText('villain_two');
  // Our results are the chip flow attributed to their seat, with the shared-hand base beside it.
  await expect(card.locator('.pc-big').first()).toContainText('OUR EDGE');
  await expect(card.locator('.pc-big').first()).toContainText('12.5');
  await expect(card.getByText(/bb\/100 vs their seat over 42 attributed hands/)).toBeVisible();
  await expect(card.locator('.pc-big').nth(1)).toContainText('9–4');
  await expect(card.getByText('CORRECTIONS IN FORCE')).toBeVisible();
  await expect(card.getByText(/fold to our bets less than the model predicts/)).toBeVisible();
  // The fitted sizing tell is named the way the API and the seat read name it.
  await expect(card.getByText(/River sizing tell \+0\.34: their bet size tracks hand strength, so they are read as betting bigger with stronger hands than the pool\./)).toBeVisible();
  // `recent` is the list of hands, newest first; without it the two extremes stand in.
  await expect(card.getByText(/RECENT HANDS/)).toBeVisible();
  await expect(card.locator('.pc-recent li')).toHaveCount(2);
  await expect(card.locator('.pc-recent li').first()).toContainText('+30,000');
  await expect(card.locator('.pc-recent li').first()).toContainText('SvanBotV10');
  await expect(card.locator('.pc-recent li').nth(1)).toContainText('-1,050');
  // A key hand recorded under a live seat replays; one under a retired name is shown, not mislinked.
  await card.getByRole('button', {name:'Replay'}).click();
  const replay = page.getByRole('dialog', {name:'Hand replay'});
  await expect(replay).toBeVisible();
  await expect(replay).toContainText('h-1aaaaa');
  await page.keyboard.press('Escape');
  await expect(replay).toHaveCount(0);
  await expect(card).toBeVisible();
  await expect(card.getByText('no replay')).toBeVisible();
  await card.getByRole('button', {name:'Close scout view'}).click();
  await expect(card).toHaveCount(0);
});

test('a scout view with almost no data still opens, says so, and closes', async ({ page }) => {
  // Regression test: the drawer it replaced once had an unpositioned "chart-empty" placeholder that
  // escaped to the fixed backdrop and swallowed clicks on the close button. The merged view has no
  // such child, and the thin payload must stay closable.
  await page.route('**/api/bots/0/opponents', async route => {
    await route.fulfill({json: [{name:'NewPlayer', style:'Observed opponent', evidence_hands:2,
      vpip:{value:.5,samples:2}, pfr:{value:.5,samples:2}}]});
  });
  await page.route('**/api/players/NewPlayer/card', route => route.fulfill({json:{
    name:'NewPlayer', style:'Observed opponent', advice:'Continue collecting public actions.', hands_observed:2, confidence:0.1,
    read:scoutRates, league:scoutRates, corrections:{fold_offset:null, response_ratio:null},
    vs_us:{hands:2, net:0, ev_net:0, bb100:null, confidence:null, ev_bb100:null, ev_confidence:null, won_pots:0, lost_pots:0, by_bot:[], form:[], series:[]},
  }}));
  await page.goto('/');
  await page.getByRole('button', {name:/NewPlayer/}).click();
  const card = page.getByRole('dialog', {name:'Scout view: NewPlayer'});
  await expect(card).toBeVisible();
  await expect(card.getByText('too few shared hands')).toBeVisible();
  await expect(card.getByText('No per-player correction yet')).toBeVisible();
  await card.getByRole('button', {name:'Close scout view'}).click({timeout: 3000});
  await expect(card).toHaveCount(0);
});

test('a table event updates the live table at once, without waiting for a snapshot', async ({page}) => {
  // 0211: a fake realtime stream the test drives; the page never refetches /api/state meanwhile.
  await page.addInitScript(() => {
    const w = window as unknown as {__svStream: Record<string, ((e: MessageEvent) => void)[]>};
    w.__svStream = {};
    window.EventSource = class DrivenEventSource {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener(kind: string, fn: (e: MessageEvent) => void) { (w.__svStream[kind] ||= []).push(fn); }
      close() {}
    } as unknown as typeof EventSource;
  });
  let stateRequests = 0;
  await page.route('**/api/state', async route => {
    stateRequests++;
    const state = await (await route.fetch()).json();
    Object.assign(state.bots[0], {connected:true, mode:'playing', hand_id:'h1', street:'flop', pot:400, hero_seat:0, board:['9c','8h','3h'],
      seats:[{seat:0, name:'TestBot', stack:2000, bet:0}, {seat:2, name:'Villain', stack:1800, bet:0, last_action:'check'}]});
    await route.fulfill({json:state});
  });
  await page.goto('/');
  const pot = page.locator('.table-panel .pot b');
  await expect(pot).toHaveText('400');
  const requestsBefore = stateRequests;
  await page.evaluate(() => {
    const w = window as unknown as {__svStream: Record<string, ((e: MessageEvent) => void)[]>};
    const bot = {slot:0, name:'TestBot', connected:true, mode:'playing', status:'playing', hand_id:'h1', street:'flop', pot:1250, hero_seat:0, board:['9c','8h','3h'], hole:[],
      seats:[{seat:0, name:'TestBot', stack:2000, bet:0}, {seat:2, name:'Villain', stack:950, bet:850, last_action:'raise'}]};
    for (const fn of w.__svStream.table || []) fn(new MessageEvent('table', {data: JSON.stringify({slot:0, bot})}));
  });
  await expect(pot).toHaveText('1,250');
  await expect(page.locator('.table-panel .seat', {hasText:'Villain'}).locator('.seat-action')).toHaveText(/raise/i);
  expect(stateRequests).toBe(requestsBefore);
  // Metrics stay from the last snapshot (the table payload carries none).
  await expect(page.getByText('NET WINNINGS', {exact:false}).first()).toBeVisible();
});

test('the winnings chart shows the all-in EV line, rate and luck', async ({page}) => {
  // 0213: all-in luck removed from the curve; the fixture has run 2,000 chips below EV.
  await fleet(page, state => {
    Object.assign(state.bots[0].metrics, {
      net_chips: 1000, bb100: 50, confidence: 120, ev_net_chips: 3000, ev_bb100: 150, ev_confidence: 90, luck_chips: -2000,
      series: [{hand:1,total:0,showdown:0,other:0,ev:0},{hand:100,total:1000,showdown:600,other:400,ev:3000}],
    });
  });
  await page.goto('/');
  await expect(page.locator('.ev-line').first()).toBeAttached();
  await expect(page.getByText('All-in EV', {exact:true}).first()).toBeVisible();
  await expect(page.getByText('ALL-IN EV RATE', {exact:true}).first()).toBeVisible();
  await expect(page.locator('.ev-stats').first()).toContainText('+150');
  await expect(page.locator('.ev-stats').first()).toContainText('-2,000');
});

test('clicking an opponent seat opens their scout view', async ({page}) => {
  // 0217/0296: a sporty scouting card with style, rates against the league and our record.
  await fleet(page, state => {
    Object.assign(state.bots[0], {connected:true, mode:'playing', hand_id:'h', street:'flop', pot:300, hero_seat:0, board:['9c','8h','3h'],
      seats:[{seat:0, name:'TestBot', stack:2000, bet:0}, {seat:2, name:'GammaBot', stack:1800, bet:0, last_action:'check'}]});
  });
  const rates = {vpip:0.3, pfr:0.2, open_raise:0.2, limp:0.05, three_bet:0.09, call_open:0.2, fold_to_3bet:0.5, four_bet:0.05, fold_to_4bet:0.4, cbet:0.7,
    fold_to_cbet:0.5, wtsd:0.3, won_showdown:0.52, river_bluff:0.25, bet_first:[0.4,0.3,0.3], fold_vs_bet:[0.4,0.5,0.6], raise_vs_bet:[0.1,0.08,0.05], vpip_pos:[0.2,0.35,0.3], open_pos:[0.15,0.3,0.2]};
  await page.route('**/api/players/GammaBot/card', route => route.fulfill({json:{
    name:'GammaBot', avatar_url:null, style:'Loose-aggressive', advice:'Value bet thinner; they call down.', hands_observed:8210, confidence:0.99,
    read:{...rates, vpip:0.47}, league:rates, corrections:{fold_offset:-0.64, response_ratio:null}, leaderboard:{rank:3, score:431902, hands:9187},
    vs_us:{hands:8210, net:243180, ev_net:468420, bb100:148.1, confidence:271.5, ev_bb100:285.3, ev_confidence:213.6, won_pots:3061, lost_pots:2352,
      biggest_win:{hand_id:'h-7wwwwwww', net:28000, pot:57000, bot:'Svanar'}, biggest_loss:{hand_id:'h-8lllllll', net:-98750, pot:198400, bot:'SurSvan'}, by_bot:[{bot:'Svanar', hands:1940, net:86000}],
      form:['W','L','W','W','='], series:[{hand:1, net:0, ev:0},{hand:8210, net:243180, ev:468420}]},
    // 0280: the head-to-head read, which can disagree in sign with the shared table result.
    vs_seat:{hands:8210, bb_per_100:-402.0, low_95:-1524.6, high_95:-117.3, beats_us:true, we_beat:false},
  }}));
  await page.goto('/');
  await page.getByRole('button', {name:'Open scout view: GammaBot'}).first().click();
  const card = page.getByRole('dialog', {name:'Scout view: GammaBot'});
  await expect(card.getByRole('heading', {name:'GammaBot'})).toBeVisible();
  await expect(card.getByText('#3 · 431,902 pts')).toBeVisible();
  // OUR EDGE is the chip flow attributed to their seat, not the table result the card used to show.
  await expect(card.locator('.pc-big').first()).toContainText('OUR EDGE');
  await expect(card.locator('.pc-big').first()).toContainText('-402');
  await expect(card.getByText(/vs their seat over 8,210 attributed hands · 95% -1,525\.\.-117/)).toBeVisible();
  // 0296: the shared-table result tile is gone; the shared hands now show as counts, not a result.
  await expect(card.getByText(/pots with them in · theirs from us · 8,210 hands they were dealt into/)).toBeVisible();
  await expect(card.locator('.pc-big').nth(1)).toContainText('3,061–2,352');
  await expect(card.locator('.pc-pill')).toHaveCount(5);
  await expect(card.locator('.pc-stat').first()).toContainText('47%');
  await expect(card.getByText(/fold to our bets less than the model predicts/)).toBeVisible();
  await expect(card.getByText(/BIGGEST HANDS/)).toBeVisible();
  await expect(card.locator('.pc-recent li')).toHaveCount(2);
  await page.setViewportSize({width:320, height:800});
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.keyboard.press('Escape');
  await expect(card).toHaveCount(0);
});

test('the decision accuracy panel grades the analyst re-solves', async ({page}) => {
  // 0220: chess-style grades of big decisions.
  const report = (acc: number, grades: number[]) => ({decisions: grades.reduce((a, b) => a + b, 0), accuracy: acc, grades, mean_loss_bb: 0.02});
  await page.route('**/api/accuracy', route => route.fulfill({json:{days:7, fleet: report(98.4, [900, 60, 20, 12, 8]),
    bots:[{bot:'TestBot', report: report(98.4, [900, 60, 20, 12, 8])}], streets:[],
    worst:[{ts: 1, bot:'TestBot', hand_id:'h', street:'river', live_action:'call', deep_action:'fold', loss_bb: 41.2, pot_bb: 60, grade:'blunder'}]}}));
  await page.goto('/');
  const panel = page.locator('[data-widget="accuracy"]');
  await expect(panel.locator('.gr-hero')).toContainText('98.4');
  await expect(panel.getByText('Blunder 8')).toBeVisible();
  await expect(panel.getByText(/call → deep fold/)).toBeVisible();
});

test('the wiring table shows what each component is worth, its sample and its age', async ({page}) => {
  // 0316: "are we really wired up fully?" answered with a measurement, most valuable component first.
  const row = (component: string, changed: number, share_pct: number, cost_bb: number, max_bb: number) => ({component, changed, share_pct, cost_bb, max_bb});
  let stale = false;
  await page.route('**/api/wiring', route => route.fulfill({json:{available: true, age_secs: stale ? 3 * 86_400 : 5_400, stale, report: {
    at: 1, sample: 150, unstable: 0, exact: 140, exact_with_current: 146, carrying_corrections: 40, v3: 40, v3_exact: 40, chosen_not_best: 1,
    rows: [row('response network', 5, 3.3, 0.61, 41), row('per-player stats (population only)', 18, 12, 10.6, 190), row('preflop fold calibration', 0, 0, 0, 0)],
    calibration: [{street: 'preflop', decisions: 3227, flipped: 1794, share_pct: 55.6, main: 'fold -> call', main_count: 1030}]}}}));
  await page.goto('/');
  const panel = page.locator('[data-widget="wiring"]');
  await expect(panel.getByText(/Measured 2 h ago on the newest 150 recorded big decisions/)).toBeVisible();
  await expect(panel.locator('tbody tr').first()).toContainText('per-player stats');
  await expect(panel.locator('tbody tr').first()).toContainText('12%');
  await expect(panel.locator('tr.wiring-idle')).toContainText('none');
  await expect(panel.getByText(/40 of 40 records that carry the live inputs replay exactly/)).toBeVisible();
  // 0332: the calibration count over every decision, where the big-decision table cannot see it.
  await expect(panel.locator('.wiring-calibration tbody tr').first()).toContainText('56%');
  await expect(panel.locator('.wiring-calibration tbody tr').first()).toContainText('fold -> call');
  stale = true;
  await page.reload();
  await expect(panel.getByText(/3 days ago.*has not re-measured it/)).toBeVisible();
});

test('the wiring table says why it has nothing to show instead of an empty table', async ({page}) => {
  await page.route('**/api/wiring', route => route.fulfill({json:{available: false, reason: 'the analyst has not measured the wiring table yet'}}));
  await page.goto('/');
  const panel = page.locator('[data-widget="wiring"]');
  await expect(panel.getByText('Unavailable: the analyst has not measured the wiring table yet.')).toBeVisible();
  await expect(panel.locator('table')).toHaveCount(0);
});

test('the quiz grades a pick against the bot and keeps a streak', async ({page}) => {
  // 0220: "What would Svanbot do?" on a recorded decision.
  await page.route('**/api/quiz', route => route.fulfill({json:{id: 1, bot:'Svanar', street:'turn', hole:['As','Kd'], board:['2c','7d','9h','Js'], pot: 1000, to_call: 0, bb: 20,
    opponents: 1, bot_action:'check', options:[
      {action:'check', amount:null, ev_bb:12, loss_bb:0, grade:'best', accuracy:100, chosen_by_bot:true},
      {action:'raise', amount:500, ev_bb:8, loss_bb:4, grade:'good', accuracy:70, chosen_by_bot:false},
      {action:'raise', amount:1000, ev_bb:-10, loss_bb:22, grade:'blunder', accuracy:5, chosen_by_bot:false}]}}));
  await page.addInitScript(() => localStorage.removeItem('svan-quiz-score:v1'));
  await page.goto('/#quiz');
  await expect(page.getByRole('heading', {name:'What would Svanbot do?'})).toBeVisible();
  await page.getByRole('button', {name:/Raise to 50/}).click();
  await expect(page.locator('.quiz-verdict')).toContainText('Blunder');
  await expect(page.locator('.quiz-verdict')).toContainText('gave up 22');
  await expect(page.getByText('Svanbot chose this')).toBeVisible();
  await expect(page.locator('.quiz-score')).toContainText('STREAK0');
  await page.getByRole('button', {name:'Next spot'}).click();
  await page.getByRole('button', {name:/^Check/}).click();
  await expect(page.locator('.quiz-verdict')).toContainText('You found the best play');
  await expect(page.locator('.quiz-score')).toContainText('STREAK1');
  await page.setViewportSize({width:320, height:800});
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('per-opponent reads show each fit, its gate and the corrected opponents', async ({page}) => {
  // 0222/0223: live fits say LIVE, a gated fit says why it waits, an unfitted one says so.
  await page.route('**/api/intel', route => route.fulfill({json: {
    fits: [
      {id: 'fold', title: 'Fold calibration', reads: 'How often each opponent folds', stored: true, active: true, evidence: {gain_mnats: 4.47, half_width_mnats: 1.24, n: 23695}, installed: 140},
      {id: 'response', title: 'Response correction', reads: 'Each opponent answering a bet', stored: false, active: false, evidence: null, installed: 0},
      {id: 'sizing', title: 'River sizing tells', reads: 'How river bet size tracks strength', stored: true, active: false, evidence: {gain_mnats: 0.56, half_width_mnats: 1.82, n: 2378}, installed: 0},
    ],
    corrected_opponents: 140,
    opponents: [{name: 'GammaBot', hands: 9880, fold_offset: -0.64, response_ratio: null, size_tell: null}],
  }}));
  await page.goto('/');
  const panel = page.locator('[data-widget="intel"]');
  await expect(panel.getByText('LIVE · 140 OPPONENTS')).toBeVisible();
  await expect(panel.getByText('NOT FITTED YET')).toBeVisible();
  await expect(panel.getByText('GATED · NOT INSTALLED')).toBeVisible();
  await expect(panel.getByText(/\+0\.56 ± 1\.82 mnats per sample on 2,378: installed only once/)).toBeVisible();
  await expect(panel.getByText(/folds less \(-0\.64\)/)).toBeVisible();
});

test('per-opponent reads say so when the server cannot answer', async ({page}) => {
  await page.route('**/api/intel', route => route.fulfill({status: 500, json: {detail: 'store unreadable (player_fold.v1): database is locked'}}));
  await page.goto('/');
  await expect(page.locator('[data-widget="intel"]').getByText(/Unavailable: store unreadable/)).toBeVisible();
});

test('the overview says what the fleet is doing, and a stale #about lands on the dashboard', async ({ page }) => {
  // 0298/0294: the header slogan and the About page are gone; the status line names the bot, what it
  // is doing (the live table, linked) and how many bots are online, separated by · not /.
  // The status string truncates the id to 8 chars; the link needs the full uuid, which rides in
  // `table_id`, so the two are set separately here exactly as `state.rs` sends them.
  const tableId = '50ea6d9a-3f2b-4c1d-8e7a-6b0f5c9d4e21';
  await fleet(page, state => {
    state.bots.forEach(b => Object.assign(b, {connected: true, mode: 'playing', status: 'playing at table 50ea6d9a', table_id: tableId}));
  });
  await page.goto('/');
  const overview = page.locator('.overview-title');
  await expect(overview).toContainText('TestBot · playing at table 50ea6d9a · 5 of 5 bots online');
  // The table's own arena page: label short, href the untruncated id.
  await expect(overview.getByRole('link', {name: '50ea6d9a'})).toHaveAttribute('href', `https://openpoker.ai/arena/${tableId}`);
  await expect(page.getByText('SVANBOT TEN')).toHaveCount(0);
  await expect(page.locator('footer')).not.toContainText('Measured decisions');
  await page.goto('/#about');
  await expect(overview).toBeVisible();
  await expect(page.locator('.about-page')).toHaveCount(0);
});

test('a failed store read is named, not served as zeros (0326)', async ({ page }) => {
  // The server sends the last good reading with stale/error when the store read failed; the
  // dashboard must say why rather than let a bot at a table read as one with 0 hands.
  await fleet(page, state => {
    Object.assign(state.bots[0].metrics, {
      error: 'store unreadable (bot results): disk I/O error',
      stale: true, hands: 4321, net_chips: 123456,
    });
  });
  await page.goto('/');
  const note = page.locator('.overview .stale-note');
  await expect(note).toBeVisible();
  await expect(note).toContainText('Store read failed (store unreadable (bot results): disk I/O error)');
  await expect(note).toContainText('last good reading, not zeros');
  await expect(page.locator('.overview-stats')).toContainText('4,321');
  await expect(page.locator('.overview-stats')).toContainText('+123,456');

  // The fleet grid marks the same bot, so a stale seat is never read as a fresh one.
  await page.getByRole('button', { name: 'Watch all' }).click();
  await expect(page.locator('.fleet-footer .footnote.amber').first()).toContainText('last good reading');
});

test('every panel explains itself when its information button is clicked (0306)', async ({ page }) => {
  // The info button renders panelHelp[title]; a panel with no entry silently shows an empty box.
  await page.goto('/');
  const buttons = page.getByRole('button', {name: /^About /});
  const count = await buttons.count();
  expect(count, 'the board has panels with information buttons').toBeGreaterThan(15);
  const silent: string[] = [];
  for (let i = 0; i < count; i++) {
    const button = buttons.nth(i);
    const label = (await button.getAttribute('aria-label')) ?? '';
    await button.scrollIntoViewIfNeeded();
    await button.click();
    const explanation = page.getByRole('region', {name: `${label.replace(/^About /, '')} explanation`});
    const text = (await explanation.locator('p').first().innerText()).trim();
    if (!text || text === 'undefined') silent.push(label);
    await button.click();
  }
  expect(silent, 'panels whose information button explains nothing').toEqual([]);
});

test('every name is clickable: an opponent opens their scout view, our bot takes over the dashboard', async ({page}) => {
  // 0297: one PlayerName everywhere a name appears.
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await fleet(page);
  await page.route('**/api/rivals', route => route.fulfill({json: {min_hands: 150, opponents: 2,
    nemeses: [{name: 'Villain', bb_per_100: -120, low_95: -300, high_95: 60, hands: 900, beats_us: false, we_beat: false, avatar_url: null}],
    donors: [{name: 'Donor', bb_per_100: 240, low_95: 100, high_95: 380, hands: 1200, beats_us: false, we_beat: true, avatar_url: null}]}}));
  const report = (acc: number) => ({decisions: 10, accuracy: acc, grades: [8, 1, 1, 0, 0], mean_loss_bb: 0.02});
  await page.route('**/api/accuracy', route => route.fulfill({json: {days: 7, fleet: report(97), bots: [{bot: 'TestBot3', report: report(97)}], streets: [], worst: []}}));
  await page.goto('/');
  // An opponent's name in the rivals panel opens their scout view.
  const villain = page.locator('[data-widget="rivals"]').getByRole('button', {name: 'Villain', exact: true});
  await villain.click();
  await expect(page.getByRole('dialog', {name: 'Scout view: Villain'})).toBeVisible();
  // Escape must close it even while the name still has focus: the name must not swallow other keys.
  await villain.focus();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog', {name: 'Scout view: Villain'})).toHaveCount(0);
  // One of our bots, named in the accuracy panel, becomes the selected bot instead.
  const ours = page.locator('[data-widget="accuracy"]').getByRole('button', {name: 'TestBot3', exact: true});
  await expect(ours).toHaveAttribute('title', 'Show TestBot3 on the dashboard');
  await ours.click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('svan-slot'))).toBe('2');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  // Keyboard: the same name is reachable and opens with Enter.
  await page.locator('[data-widget="rivals"]').getByRole('button', {name: 'Donor', exact: true}).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog', {name: 'Scout view: Donor'})).toBeVisible();
});

test('the docs page draws its three diagrams and keeps each text version', async ({page}) => {
  // 0300: `diagram:` fences in docs/GUIDE.md render as SVG; the ASCII stays one click away.
  await page.setViewportSize({width: 390, height: 900});
  await page.goto('/#docs');
  const figures = page.locator('.dg-figure');
  await expect(figures).toHaveCount(3);
  for (const label of [/Processes and data/, /decision path/, /learner loop/]) {
    await expect(page.getByRole('img', {name: label})).toBeVisible();
  }
  await figures.first().getByText('Text version').click();
  await expect(figures.first().locator('pre')).toContainText('svanbot10.db');
  expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
});
