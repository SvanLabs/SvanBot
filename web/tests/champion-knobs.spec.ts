import { test, expect, type Page } from '@playwright/test';
import type { Snapshot } from '../src/types';

// The Champion profile's parameter rows (#322). Two things were wrong with them, and both are here.
//
// The panel drew its rows from a literal typed into `web/src/training.tsx` — a key, a min, a max and
// an explanation per knob — with nothing tying it to the knobs the learner actually searches. A knob
// the search gained never appeared, and a bound it widened left the bar measuring against the old
// one. The rows are `training.knobs` now, so these tests serve a catalogue the running server would
// never send and check that the panel drew it anyway.
//
// And a knob whose value the payload did not carry fell back to the knob's *minimum*: an empty bar,
// which reads as "pinned to its floor", printed beside a `—` saying the value was unknown. The two
// halves of the cell said different things and the bar was the half that lied.

// The Champion profile is on the full board, as in tests/dashboard.spec.ts (0237).
test.beforeEach(async ({ page }) => { await page.addInitScript(() => localStorage.setItem('svan-view', 'all')); });

/** Serve the real snapshot with the Champion profile's knob catalogue and values rewritten. The
 *  event stream is quieted first, as `fleet()` in tests/dashboard.spec.ts does: it pushes the real
 *  snapshot, which would put the server's own catalogue back a moment after the page settles. */
async function withKnobs(page: Page, mutate: (state: Snapshot) => void) {
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
    const state: Snapshot = await (await route.fetch()).json();
    mutate(state);
    await route.fulfill({json: state});
  });
}

/** The track of one knob row, found by the label the server gave it. */
const track = (page: Page, label: string) => page.getByRole('img', {name: new RegExp(`^${label}: `)});

/** Where a track's fill ends, as a share of the track, so a bar can be read as a value. */
async function fillShare(page: Page, label: string) {
  const bar = track(page, label);
  const [outer, inner] = [await bar.boundingBox(), await bar.locator('i').boundingBox()];
  expect(outer, 'the track is on the page').not.toBeNull();
  expect(inner, 'the track has a bar').not.toBeNull();
  return inner!.width / outer!.width;
}

test('the profile draws the knobs the server defines, over the bounds it defines', async ({ page }) => {
  // A catalogue no hand-written copy could contain: one knob, renamed, with bounds and a sentence
  // that exist nowhere in the dashboard's source.
  await withKnobs(page, state => {
    state.training.knobs = [{key: 'fold_scale', label: 'Folding tendency', min: 0, max: 2, decimals: 3,
      default: 1, description: 'A sentence only the running server could have supplied.'}];
    state.training.champion.fold_scale = 0.5;
  });
  await page.goto('/');

  const profile = page.locator('.profile');
  await expect(profile.locator('.knob')).toHaveCount(1);
  await expect(profile.getByText('Folding tendency', {exact: true})).toBeVisible();
  await expect(profile.getByText('A sentence only the running server could have supplied.')).toBeVisible();
  // `decimals` is the server's too: three places, not the two the panel used to hard-code.
  await expect(profile.locator('.knob b')).toHaveText('0.500');
  // 0.5 over the 0..2 the server sent is a quarter of the track. Against the bounds the dashboard
  // used to carry for this knob (0.4..1.2) the same value would have drawn an empty bar.
  expect(await fillShare(page, 'Folding tendency')).toBeGreaterThan(0.2);
  expect(await fillShare(page, 'Folding tendency')).toBeLessThan(0.3);
  await expect(track(page, 'Folding tendency')).toHaveAttribute('aria-label', 'Folding tendency: 0.500 of 0.000 to 2.000');
});

test('a knob the payload has no value for draws no bar at all', async ({ page }) => {
  await withKnobs(page, state => {
    state.training.knobs = [
      {key: 'call_margin', label: 'Call margin', min: -0.1, max: 0.08, decimals: 2, default: 0,
        description: 'Minimum EV a call must show over folding, as a share of the pot.'},
      {key: 'fold_scale', label: 'Fold-probability scale', min: 0.4, max: 1.2, decimals: 2, default: 1,
        description: 'Multiplier on every modeled opponent fold probability.'},
    ];
    // The champion reports one of them and not the other.
    delete state.training.champion.call_margin;
    state.training.champion.fold_scale = 0.8;
  });
  await page.goto('/');

  // The unknown one: the number says unknown and the track agrees, because there is no bar on it.
  // Found by its printed label rather than the track's description, so the assertion below is about
  // the bar and not about the markup around it — an empty bar is exactly what this used to draw.
  const unknown = page.locator('.knob', {has: page.getByText('Call margin', {exact: true})});
  await expect(unknown.locator('b')).toHaveText('—');
  await expect(unknown.locator('.knob-track i')).toHaveCount(0);
  await expect(track(page, 'Call margin')).toHaveAttribute('aria-label', 'Call margin: not reported');

  // Not "every bar is gone": the knob beside it is reported, so it is drawn, at half of 0.4..1.2.
  await expect(page.locator('.knob', {has: track(page, 'Fold-probability scale')}).locator('b')).toHaveText('0.80');
  expect(await fillShare(page, 'Fold-probability scale')).toBeGreaterThan(0.45);
  expect(await fillShare(page, 'Fold-probability scale')).toBeLessThan(0.55);
});

test('every knob marks its shipped default, so a value can be read as high or low', async ({ page }) => {
  // "0.55 in a 0.4-1.2 range" is not readable without a reference on the track (#322).
  await withKnobs(page, state => {
    state.training.knobs = [{key: 'fold_scale', label: 'Fold-probability scale', min: 0, max: 1, decimals: 2,
      default: 0.75, description: 'Multiplier on every modeled opponent fold probability.'}];
    state.training.champion.fold_scale = 0.25;
  });
  await page.goto('/');

  const bar = track(page, 'Fold-probability scale');
  const marker = bar.locator('.knob-default');
  await expect(marker).toHaveAttribute('title', 'Default 0.75 · searched over 0.00 to 1.00');
  const [outer, mark] = [await bar.boundingBox(), await marker.boundingBox()];
  const share = (mark!.x - outer!.x) / outer!.width;
  expect(share, 'the default sits at 0.75 of the track').toBeGreaterThan(0.7);
  expect(share).toBeLessThan(0.8);
  // And the champion's bar is well to the left of it, which is the whole point of having the mark.
  expect(await fillShare(page, 'Fold-probability scale')).toBeLessThan(share);
});

test('the real server supplies a catalogue the profile can draw', async ({ page }) => {
  // The mocks above prove the panel renders what it is given; this one proves the fleet gives it
  // something. Every row the sandboxed server sends has a label, a sentence and a bar on its track.
  await page.goto('/');
  const rows = page.locator('.profile .knob');
  await expect(rows.first()).toBeVisible();
  expect(await rows.count()).toBeGreaterThanOrEqual(17);
  for (const row of await rows.all()) {
    await expect(row.locator('label')).not.toBeEmpty();
    await expect(row.locator('.knob-description')).not.toBeEmpty();
    await expect(row.locator('.knob-default')).toHaveCount(1);
    await expect(row.locator('b')).not.toHaveText('—');
    await expect(row.locator('.knob-track i')).toHaveCount(1);
  }
});
