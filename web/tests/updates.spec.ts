import { test, expect, type Page } from '@playwright/test';

// Panel tests run on the full board (the All view); tests/views.spec.ts covers the views (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

// One-click update (0236): the panel offers what GitHub holds, and a run shows a stage-weighted
// progress bar with the bots still playing, then the swap. The API is mocked per test.
const releases = {
  installed: { commit: 'aaaaaaa', at: '2026-09-26T01:30:28+02:00', subject: 'feat: before' },
  head: { commit: 'aaaaaaa', subject: 'feat: before' },
  behind: 2,
  dirty: false,
  update_available: true,
  build: { commit: 'aaaaaaa', version: '10.0.0' },
  changelog: [
    { commit: 'ccccccc', subject: 'feat: size-scaled overbet call shift', group: 'Features' },
    { commit: 'bbbbbbb', subject: 'perf: tests rebuild in seconds', group: 'Performance' },
  ],
  remote: { source: 'origin/main', commit: 'ccccccc', behind: 2, checked_at: Date.now() / 1000 - 60, error: null },
};

const stage = (name: string, state: string, seconds: number, expected: number) => ({ name, state, seconds, expected });

function progress(state: 'idle' | 'running' | 'failed' | 'installed', over: Record<string, unknown> = {}) {
  const base = {
    state, running: state === 'running', percent: 0, elapsed: null, eta: null, from: 'aaaaaaa', commit: null, message: null,
    swap: { target: null, fleet: 'aaaaaaa', fleet_done: false, learner: 'aaaaaaa', analyst: 'aaaaaaa', workers: [] },
    bots_playing: 5, bots_total: 5, log: ['== fetching origin/main'],
    stages: ['fetch', 'snapshot', 'lint', 'test', 'build', 'dashboard', 'install'].map(n => stage(n, 'pending', 0, 30)),
  };
  return { ...base, ...over };
}

async function mockReleases(page: Page, runs: object[]) {
  await page.route('**/api/releases', route => route.fulfill({ json: releases }));
  let i = 0;
  await page.route('**/api/releases/progress', route => route.fulfill({ json: runs[Math.min(i++, runs.length - 1)] }));
}

test('an available update names its commits and asks before installing', async ({ page }) => {
  await mockReleases(page, [progress('idle')]);
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await expect(panel.getByText('2 updates to install')).toBeVisible();
  await expect(panel.getByText('feat: size-scaled overbet call shift')).toBeVisible();
  await panel.getByRole('button', { name: 'Update', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Confirm update' });
  await expect(dialog.getByText('Install ccccccc?')).toBeVisible();
  await expect(dialog.getByText(/keep playing the whole time/)).toBeVisible();
});

test('a running update shows a stage-weighted bar, time left and the bots still playing', async ({ page }) => {
  const running = progress('running', {
    percent: 60, elapsed: 250, eta: 160,
    stages: [stage('fetch', 'done', 3, 4), stage('snapshot', 'done', 7, 6), stage('lint', 'done', 45, 40), stage('test', 'done', 55, 50),
      stage('build', 'running', 140, 280), stage('dashboard', 'pending', 0, 15), stage('install', 'pending', 0, 5)],
  });
  const installed = progress('installed', {
    percent: 100, elapsed: 420, commit: 'ccccccc',
    stages: ['fetch', 'snapshot', 'lint', 'test', 'build', 'dashboard', 'install'].map(n => stage(n, 'done', 10, 10)),
    swap: { target: 'ccccccc', fleet: 'ccccccc', fleet_done: true, learner: 'aaaaaaa', analyst: 'ccccccc', workers: [] },
  });
  await mockReleases(page, [running, running, installed]);
  await page.goto('/');
  const bar = page.getByRole('progressbar', { name: 'Update progress' });
  await expect(bar).toHaveAttribute('aria-valuenow', '60');
  const box = page.locator('.update-progress');
  await expect(box.getByText('Updating')).toBeVisible();
  await expect(box.getByText(/60% · 4m 10s elapsed · about 2m 40s left/)).toBeVisible();
  await expect(box.getByText('5 of 5 bots playing — play continues during the update')).toBeVisible();
  // The label names what the stage does (996b5a0 made the build stage lint and both builds).
  await expect(box.locator('li.running')).toContainText('Lint, test build and release build');
  await expect(box.locator('li.done')).toHaveCount(4);
  // The next polls: installed and swapped.
  await expect(box.getByText('Update complete')).toBeVisible({ timeout: 10_000 });
  await expect(bar).toHaveAttribute('aria-valuenow', '100');
  await expect(box.getByText(/learner next cycle/)).toBeVisible();
});

test('a failed update names the stage and says the fleet kept playing', async ({ page }) => {
  const failed = progress('failed', {
    elapsed: 95, message: 'release failed; the fleet keeps playing the installed build',
    stages: [stage('fetch', 'done', 3, 4), stage('snapshot', 'done', 7, 6), stage('lint', 'done', 45, 40), stage('test', 'failed', 40, 50),
      stage('build', 'pending', 0, 280), stage('dashboard', 'pending', 0, 15), stage('install', 'pending', 0, 5)],
    log: ['== testing', '---- sv10_policy ----', 'test.py: 1 failed'],
  });
  await mockReleases(page, [failed]);
  await page.goto('/');
  const box = page.locator('.update-progress');
  await expect(box.getByText('Update failed')).toBeVisible();
  await expect(box.getByText(/Failed at Test\./)).toBeVisible();
  await expect(box.getByText(/keeps playing the installed build/).first()).toBeVisible();
  await expect(box.getByText('test.py: 1 failed')).toBeVisible();
});

test('a saved build can be rolled back to, with the same progress and play continuing', async ({ page }) => {
  const rolling = progress('running', { percent: 40, elapsed: 8, eta: 12, stages: [stage('restore', 'running', 8, 20)] });
  await page.route('**/api/releases', route => route.fulfill({ json: releases }));
  let started = false;
  await page.route('**/api/releases/progress', route => route.fulfill({ json: started ? rolling : progress('idle') }));
  await page.route('**/api/releases/snapshots', route => route.fulfill({ json: { running: false, snapshots: [
    { commit: 'bbbbbbb', subject: 'feat: the build before', installed_at: '2026-09-26T01:30:28+02:00', current: false, readable: true },
    { commit: 'aaaaaaa', subject: 'feat: before', installed_at: '2026-09-25T00:00:08+02:00', current: true, readable: true },
    { commit: '9999999', subject: 'feat: before compression', installed_at: '2026-09-20T00:00:08+02:00', current: false, readable: false },
  ] } }));
  let posted: unknown = null;
  await page.route('**/api/releases/rollback', route => { posted = route.request().postDataJSON(); started = true; return route.fulfill({ json: { started: true } }); });
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await panel.getByText('Roll back to a saved build (2)').click();
  // A build that cannot read the compressed databases (0229) is listed without a Roll back button.
  const old = panel.locator('li', { hasText: '9999999' });
  await expect(old.getByText('reads only uncompressed data')).toBeVisible();
  await expect(old.getByRole('button', { name: 'Roll back' })).toHaveCount(0);
  await panel.locator('li', { hasText: 'bbbbbbb' }).getByRole('button', { name: 'Roll back' }).click();
  const dialog = page.getByRole('dialog', { name: 'Confirm rollback' });
  await expect(dialog.getByText('Reinstall bbbbbbb?')).toBeVisible();
  await dialog.getByRole('button', { name: 'Roll back' }).click();
  expect(posted).toEqual({ commit: 'bbbbbbb' });
  const box = page.locator('.update-progress');
  await expect(box.getByText('Rolling back')).toBeVisible();
  await expect(box.locator('li.running')).toContainText('Restore saved build');
  await expect(box.getByText(/5 of 5 bots playing/)).toBeVisible();
});

// 0320: one panel, two runs. A failed check must date the commit list instead of passing it off as
// today's, and the mechanism's own wording must not be the message.
test('a failed check dates the list and says what to do, not the mechanism', async ({ page }) => {
  const checked = Date.now() / 1000 - 240;
  await page.route('**/api/releases', route => route.fulfill({ json: { ...releases, remote: {
    source: 'origin/main', commit: 'ccccccc', behind: 2, checked_at: checked, fetched_at: checked - 4 * 3600,
    error: 'update: could not fetch origin/main (network or credentials); nothing changed',
  } } }));
  await page.route('**/api/releases/progress', route => route.fulfill({ json: progress('idle') }));
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await expect(panel.getByText(/^check failed/)).toBeVisible();
  await expect(panel.getByText(/checked 4 min ago/)).toBeVisible();
  // The line names what an operator can act on, and keeps the tool's words in the tooltip.
  await expect(panel.getByText(/GitHub could not be reached from this host/)).toBeVisible();
  await expect(panel.locator('p.negative')).toHaveAttribute('title', /could not fetch origin\/main/);
  // And the list below it is dated as the last successful fetch, not as today's.
  await expect(panel.getByText(/This list is the one the last successful check fetched — 4 h ago\. Today's commits are not in it\./)).toBeVisible();
});

// 0320: the run that finished and the run that cannot start are different runs, and the panel has to
// say which is which — the checkout is a playing machine, so "commit or discard" is not the message.
test('a blocked next update is named apart from the run that finished', async ({ page }) => {
  const done = progress('installed', {
    percent: 100, elapsed: 79, finished_at: Date.now() / 1000 - 300, commit: 'ccccccc',
    stages: ['fetch', 'snapshot', 'build', 'test', 'dashboard', 'install'].map(n => stage(n, 'done', 10, 10)),
    swap: { target: 'ccccccc', fleet: 'ccccccc', fleet_done: true, learner: 'ccccccc', analyst: 'ccccccc', workers: [] },
  });
  await page.route('**/api/releases', route => route.fulfill({ json: { ...releases, dirty: true } }));
  await page.route('**/api/releases/progress', route => route.fulfill({ json: done }));
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await expect(panel.getByText('Next update')).toBeVisible();
  const blocked = panel.getByText('Blocked — uncommitted build inputs');
  await expect(blocked).toBeVisible();
  await expect(blocked).toHaveAttribute('title', /will not start while crates\/, web\/ or Cargo files have uncommitted changes/);
  // The card above is the run that already finished, and says when.
  await expect(panel.getByText('Update complete')).toBeVisible();
  await expect(panel.getByText('aaaaaaa → ccccccc in 1m 19s · finished 5 min ago')).toBeVisible();
  // An update cannot start from a dirty tree, so the button stays disabled.
  await expect(panel.getByRole('button', { name: 'Update', exact: true })).toBeDisabled();
});


test('unavailable saved builds and progress are visible and recover when signed in', async ({ page }) => {
  await page.route('**/api/releases', route => route.fulfill({ json: releases }));
  let unavailable = true;
  await page.route('**/api/releases/snapshots', route => unavailable
    ? route.fulfill({ status: 503, json: { detail: 'Saved builds offline' } })
    : route.fulfill({ json: { snapshots: [] } }));
  await page.route('**/api/releases/progress', route => unavailable
    ? route.fulfill({ status: 503, json: { detail: 'Progress offline' } })
    : route.fulfill({ json: progress('failed', { message: 'Prior build failed' }) }));
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await expect(panel.getByText(/Saved builds offline/)).toBeVisible();
  await expect(panel.getByText(/Progress offline/)).toBeVisible();
  unavailable = false;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel.getByText(/Saved builds offline/)).toHaveCount(0);
  await expect(panel.getByText(/Progress offline/)).toHaveCount(0);
  await expect(panel.getByText(/Prior build failed/)).toBeVisible();
});

test('failed progress polling labels the retained update as stale', async ({ page }) => {
  await page.clock.install();
  await page.route('**/api/releases', route => route.fulfill({ json: releases }));
  let fail = false;
  await page.route('**/api/releases/progress', route => fail
    ? route.fulfill({ status: 503, json: { detail: 'Progress refresh offline' } })
    : route.fulfill({ json: progress('running', { percent: 60 }) }));
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await expect(panel.getByRole('progressbar', { name: 'Update progress' })).toHaveAttribute('aria-valuenow', '60');
  fail = true;
  await page.clock.fastForward(2_100);
  await expect(panel.getByText(/Progress refresh offline/)).toBeVisible();
  await expect(panel.getByText(/showing data from/i)).toBeVisible();
  await expect(panel.getByRole('progressbar', { name: 'Update progress' })).toHaveAttribute('aria-valuenow', '60');
});

test('a failed saved-build refresh labels the retained rollback list as stale', async ({ page }) => {
  await page.route('**/api/releases', route => route.fulfill({ json: releases }));
  await page.route('**/api/releases/progress', route => route.fulfill({ json: progress('idle') }));
  let fail = false;
  await page.route('**/api/releases/snapshots', route => fail
    ? route.fulfill({ status: 503, json: { detail: 'Saved build refresh offline' } })
    : route.fulfill({ json: { snapshots: [{ commit: 'ddddddd', subject: 'Prior safe build', current: false }] } }));
  await page.goto('/');
  const panel = page.locator('.updates-panel');
  await panel.locator('.saved-builds summary').click();
  await expect(panel.getByText(/Prior safe build/)).toBeVisible();
  fail = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(panel.getByText(/Saved build refresh offline/)).toBeVisible();
  await expect(panel.getByText(/showing data from/i)).toBeVisible();
  await expect(panel.getByText(/Prior safe build/)).toBeVisible();
});
