import { test, expect } from '@playwright/test';
import type { Snapshot } from '../src/types';

test('candidate evidence states its recorded stack coverage', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('svan-view', 'all'));
  await page.route('**/api/events', route => route.abort());
  await page.route('**/api/state', async route => {
    const state:Snapshot = await (await route.fetch()).json();
    state.training.experiments = [{id:'stacked',status:'rejected',ts:1,population:{id:'pool',opponent_count:16,evidence:1000,
      evaluation:{id:'frozen',basis:'recorded six-seat starting stacks',sampled:256,eligible:3800,excluded:296,cutoff:10000}}}];
    await route.fulfill({json:state});
  });
  await page.goto('/');
  await expect(page.getByText('recorded six-seat starting stacks · 256 layouts · 3,800 / 4,096 eligible hands')).toBeVisible();
});
