import { test, expect } from '@playwright/test';
import type { Snapshot } from '../src/types';

test('recent rejections do not hide a successful promotion or double the candidate count', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const state: Snapshot = await (await route.fetch()).json();
    state.training.experiments = Array.from({length:40}, (_, id) => ({id:`rejected-${id}`, status:'rejected', ts:2, knob:'call_margin', old:0, new:.01}));
    state.training.last_promotion = {id:'successful-change', status:'promoted', ts:1, knob:'open_bb', old:2.5, new:2.25, rationale:'Won fresh-deal confirmation.'};
    state.training.learning = [{what:'Strategy search',updated:1,detail:'46 successful promotions · last promotion yesterday'}];
    state.training.search_funnel = {hours:24,total:969, outcomes:[{key:'search/proposed',count:485},{key:'search/halved-out',count:382},{key:'promotion/promoted',count:1}],knobs:[]};
    await route.fulfill({json:state});
  });
  await page.goto('/');
  await expect(page.getByText('46 successful promotions · last promotion yesterday')).toBeVisible();
  await expect(page.getByText('485 proposed', {exact:true})).toBeVisible();
  await expect(page.locator('.funnel-head').getByText('969',{exact:true})).toHaveCount(0);
  await expect(page.getByText('LATEST SUCCESSFUL CHANGE')).toBeVisible();
  await expect(page.getByText('Won fresh-deal confirmation.')).toBeVisible();
  await expect(page.getByText('promoted',{exact:true})).toBeVisible();
  await expect(page.getByText(/Latest 6 candidate decisions · 40 retained/)).toBeVisible();
});
