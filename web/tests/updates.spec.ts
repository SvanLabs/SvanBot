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
  await mockReleases(page, [progress('idle'), rolling]);
  await page.route('**/api/releases/snapshots', route => route.fulfill({ json: { running: false, snapshots: [
    { commit: 'bbbbbbb', subject: 'feat: the build before', installed_at: '2026-09-26T01:30:28+02:00', current: false, readable: true },
    { commit: 'aaaaaaa', subject: 'feat: before', installed_at: '2026-09-25T00:00:08+02:00', current: true, readable: true },
    { commit: '9999999', subject: 'feat: before compression', installed_at: '2026-09-20T00:00:08+02:00', current: false, readable: false },
  ] } }));
  let posted: unknown = null;
  await page.route('**/api/releases/rollback', route => { posted = route.request().postDataJSON(); return route.fulfill({ json: { started: true } }); });
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
