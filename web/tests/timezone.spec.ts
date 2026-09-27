import { test, expect, type Page } from '@playwright/test';

// 0295: the API sends unix seconds (or an RFC 3339 string with an offset) and only the browser
// formats them, through `time` in src/format.ts, in the viewer's own zone. This is the acceptance
// check: the same payload, the same page, one viewer west of UTC and one east of it.
const AT = Date.parse('2026-09-27T12:00:00Z') / 1000; // the mocked clock, and a stamp from "now"
const EARLIER = Date.parse('2026-09-25T22:30:00Z') / 1000; // a stamp that is another day in both zones

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  // The sandbox's real event stream would push its own log lines over the mocked snapshot (fleet()
  // in dashboard.spec.ts does the same): replace it with a quiet, already-open one.
  await page.addInitScript(() => {
    window.EventSource = class QuietEventSource {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      constructor() { setTimeout(() => this.onopen?.(new Event('open')), 0); }
      addEventListener() {}
      close() {}
    } as unknown as typeof EventSource;
  });
});

/** Serve a state whose times are known, under a fixed browser clock so "today" is a fixed day. */
async function frozenDashboard(page: Page) {
  await page.clock.setFixedTime(new Date(AT * 1000));
  await page.route('**/api/state', async route => {
    const state = await (await route.fetch()).json();
    state.updated = AT;
    state.logs = [{ id: 1, ts: EARLIER, level: 'info', slot: null, message: 'Rotation complete.' }];
    await route.fulfill({ json: state });
  });
}

/** The freshness stamp (today, so time only) and an activity-log line (another day, so date and all;
 *  `en-GB` spells September `Sept`). */
async function expectOnTheViewersClock(page: Page, zone: string, updated: string, logged: string) {
  expect(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone), 'this context really is in the viewer zone').toBe(zone);
  await expect(page.locator('.workspace-tools .updated')).toHaveText(`Updated ${updated}`);
  await expect(page.locator('.log-entry time').first()).toHaveText(logged);
}

// `Etc/GMT+8` is UTC−8: the sign is inverted in the POSIX zone names. A fixed offset keeps the
// expectation exact in any month, and Asia/Tokyo is UTC+9 all year.
test.describe('a viewer at UTC−8', () => {
  test.use({ timezoneId: 'Etc/GMT+8' });

  test('reads every stamp on their own clock', async ({ page }) => {
    await frozenDashboard(page);
    await page.goto('/');
    await expectOnTheViewersClock(page, 'Etc/GMT+8', '04:00:00', '25 Sept 14:30:00');
  });
});

test.describe('a viewer at UTC+9', () => {
  test.use({ timezoneId: 'Asia/Tokyo' });

  test('reads the same stamps on theirs, dates and all', async ({ page }) => {
    await frozenDashboard(page);
    await page.goto('/');
    await expectOnTheViewersClock(page, 'Asia/Tokyo', '21:00:00', '26 Sept 07:30:00');
  });
});
