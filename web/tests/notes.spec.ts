import { test, expect, type Page } from '@playwright/test';
import type { DashboardNotes } from '../src/types';

// The notes scratchpad (#730): one plain-text panel on the board that saves itself while it is typed
// — debounced, on blur and on the way out — to the server, with this browser's copy as what renders
// before the answer arrives and what stands when the endpoint cannot be reached. A failed save has to
// be visible and retried, never swallowed (LESSONS 24).

const TV = `http://127.0.0.1:${process.env.SV10_TEST_TV_PORT || '8787'}`;
const LOCAL_KEY = 'svan-notes:v1';

const box = (page: Page) => page.getByRole('textbox', { name: 'Operator notes' });
const state = (page: Page) => page.locator('.notes-state');

/** The notes endpoint as the board uses it: `GET` answers what is stored, `POST` reports what it was
 *  sent. `stored === null` is "nothing stored" — the browser's copy is then what shows. */
async function mockNotes(page: Page, options: { stored?: DashboardNotes | null; posted?: DashboardNotes[]; fail?: boolean } = {}) {
  const { stored = null, posted = [], fail = false } = options;
  await page.route('**/api/notes', route => {
    if (fail) return route.fulfill({ status: 503, json: { detail: 'notes store unavailable' } });
    if (route.request().method() === 'GET') return route.fulfill({ json: stored });
    const body = route.request().postDataJSON() as DashboardNotes;
    posted.push(body);
    return route.fulfill({ json: body });
  });
  return posted;
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'live'));
  await page.route('**/api/layout', route => route.fulfill({ json: null }));
});

test('typing saves the whole note a second later, and the box says so', async ({ page }) => {
  const posted = await mockNotes(page);
  await page.goto('/');
  await box(page).fill('watch the limpers: they fold to a turn barrel');
  await expect(state(page)).toHaveText('Saved');
  expect(posted.at(-1)).toEqual({ text: 'watch the limpers: they fold to a turn barrel' });
});

test('the box is an ordinary widget of the board, in Live and in All', async ({ page }) => {
  await mockNotes(page);
  await page.goto('/');
  await expect(page.locator('[data-widget="notes"] .notes-input')).toHaveCount(1);
  await page.getByRole('tab', { name: 'All' }).click();
  await expect(page.locator('[data-widget="notes"] .notes-input')).toHaveCount(1);
});

test('a note is stored on the server and is what a reload shows', async ({ page, request }) => {
  const clear = () => request.post('/api/notes', { data: { text: '' } });
  await clear();
  await page.goto('/');
  await box(page).fill('the quiet seat on the left is a calling station');
  await expect(state(page)).toHaveText('Saved');
  // Nothing of this browser is left behind: what the box shows after the reload can only be the
  // server's answer, not a copy this browser kept.
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await expect(box(page)).toHaveValue('the quiet seat on the left is a calling station');
  await expect(state(page)).toHaveText('Saved');
  await clear();
});

test('a browser holding an old copy does not put back a note the server cleared', async ({ page }) => {
  const posted = await mockNotes(page, { stored: null });
  await page.addInitScript(() => {
    localStorage.setItem('svan-notes:v1', 'a note that was cleared elsewhere');
    localStorage.setItem('svan-notes:unsaved', '0');
  });
  await page.goto('/');
  // Nothing here is unsent, so the server is the truth: it says "nothing stored" and the stale copy
  // goes, instead of the old text being pushed back over the clear.
  await expect(box(page)).toHaveValue('');
  await expect.poll(() => page.evaluate(key => localStorage.getItem(key), LOCAL_KEY)).toBe('');
  expect(posted).toEqual([]);
});

test('text an earlier session could not send is pushed on load, and a refused push says so and retries', async ({ page }) => {
  const posted: DashboardNotes[] = [];
  let pushes = 0;
  await page.addInitScript(() => {
    localStorage.setItem('svan-notes:v1', 'written while the server was away');
    localStorage.setItem('svan-notes:unsaved', '1');
  });
  await page.route('**/api/notes', route => {
    if (route.request().method() === 'GET') return route.fulfill({ status: 503, json: { detail: 'notes store unavailable' } });
    if (pushes++ === 0) return route.fulfill({ status: 503, json: { detail: 'notes store unavailable' } });
    const body = route.request().postDataJSON() as DashboardNotes;
    posted.push(body);
    return route.fulfill({ json: body });
  });
  await page.goto('/');
  await expect(box(page)).toHaveValue('written while the server was away');
  // The push failed, and the panel says so instead of sitting unsaved and silent.
  await expect(state(page)).toHaveText('Not saved — retrying…');
  await expect(state(page)).toHaveText('Saved');
  expect(posted).toEqual([{ text: 'written while the server was away' }]);
});

test('an endpoint that cannot be reached falls back to this browser, and loses nothing typed', async ({ page }) => {
  await mockNotes(page, { fail: true });
  await page.goto('/');
  await box(page).fill('written while the server was away');
  // A save that failed says so, and keeps trying; it never looks saved.
  await expect(state(page)).toHaveText('Not saved — retrying…');
  expect(await page.evaluate(key => localStorage.getItem(key), LOCAL_KEY)).toBe('written while the server was away');
  await page.reload();
  await expect(box(page)).toHaveValue('written while the server was away');
});

test('a save the endpoint refused keeps trying until it lands', async ({ page }) => {
  const posted: DashboardNotes[] = [];
  let refusals = 0;
  await page.route('**/api/notes', route => {
    if (route.request().method() === 'GET') return route.fulfill({ json: null });
    if (refusals === 0) { refusals += 1; return route.fulfill({ status: 503, json: { detail: 'notes store unavailable' } }); }
    const body = route.request().postDataJSON() as DashboardNotes;
    posted.push(body);
    return route.fulfill({ json: body });
  });
  await page.goto('/');
  await box(page).fill('a note the store refuses once');
  await expect(state(page)).toHaveText('Not saved — retrying…');
  // The retry is scheduled, not forgotten: the same note goes out again and the box says so.
  await expect(state(page)).toHaveText('Saved');
  expect(posted).toEqual([{ text: 'a note the store refuses once' }]);
});

test('the last keystrokes before the page goes leave with the page', async ({ page }) => {
  const posted = await mockNotes(page);
  await page.goto('/');
  await box(page).fill('typed in the last second before leaving');
  // Before the debounce comes round, the page goes: the flush is what carries the text out.
  await page.evaluate(() => window.dispatchEvent(new Event('pagehide')));
  await expect.poll(() => posted.at(-1)).toEqual({ text: 'typed in the last second before leaving' });
});

test('the TV listener serves no notes route, and the dashboard does', async ({ request }) => {
  expect((await request.get(`${TV}/api/notes`)).status(), 'the public TV answered a dashboard route').toBe(404);
  expect((await request.post(`${TV}/api/notes`, { data: { text: 'x' } })).status()).toBe(404);
  expect((await request.get('/api/notes')).ok(), 'the dashboard must serve the route the TV refuses').toBe(true);
});

// The endpoint itself, unmocked: the sandbox's store is shared by every test in the run, so this
// stores a note and clears it again (never relies on either being there), and pins the server
// contract rather than a page render — the render is covered above with a mocked answer.
test('the notes endpoint round-trips a note and refuses a body that is not one', async ({ request }) => {
  const clear = () => request.post('/api/notes', { data: { text: '' } });
  try {
    await clear();
    expect(await (await request.get('/api/notes')).json()).toBeNull();
    // A body that is not `{text: string}` is refused and stores nothing.
    expect((await request.post('/api/notes', { data: { note: 'not the shape' } })).status()).toBe(400);
    expect(await (await request.get('/api/notes')).json()).toBeNull();
    const saved = await request.post('/api/notes', { data: { text: 'a stored note' } });
    expect(saved.ok()).toBe(true);
    expect(await (await request.get('/api/notes')).json()).toEqual({ text: 'a stored note' });
    // Empty text is the store's "nothing stored" — what a cleared box leaves behind.
    await clear();
    expect(await (await request.get('/api/notes')).json()).toBeNull();
  } finally {
    await clear();
  }
});

// The panel mounts before the login. With a token set its first request is refused, and it used to
// leave it at that: the box stayed empty after the login and the first edit replaced the stored note
// (#881). Once a session exists it asks again and shows what is stored.
test('a first request refused before the login is asked again once a session exists', async ({ page }) => {
  const posted: DashboardNotes[] = [];
  let signedIn = false;
  await page.route('**/api/notes', route => {
    if (route.request().method() !== 'GET') { posted.push(route.request().postDataJSON() as DashboardNotes); return route.fulfill({ json: route.request().postDataJSON() }); }
    return signedIn ? route.fulfill({ json: { text: 'kept on the server' } }) : route.fulfill({ status: 401, json: { detail: 'unauthorized' } });
  });
  await page.goto('/');
  await expect(box(page)).toHaveValue('');
  signedIn = true;
  await page.evaluate(() => window.dispatchEvent(new Event('sv-session')));
  await expect(box(page)).toHaveValue('kept on the server');
  expect(posted).toEqual([]);
});
