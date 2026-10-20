import { test, expect } from '@playwright/test';
import { loadEnv } from '../wallet.js';

const env = loadEnv();
const appUrl = `/?rpc=${encodeURIComponent(env.rpc_url)}&api=${encodeURIComponent(env.api_url)}`;

const KPIS = ['kpi-volume-24h', 'kpi-oi', 'kpi-traders', 'kpi-tvl', 'kpi-fees', 'kpi-insurance', 'kpi-liquidations-24h'];

test.beforeEach(async ({ page }) => {
  page.on('console', (m) => { if (m.type() === 'error') console.log('[page]', m.text()); });
});

test('analytics page shows protocol stats, then navigates back to trade', async ({ page }) => {
  await page.goto(`${appUrl}#/analytics`);
  await expect(page.getByTestId('analytics-page')).toBeVisible();
  for (const id of KPIS) {
    await expect(page.getByTestId(id)).toHaveText(/\d/);
  }
  await expect(page.getByTestId('volume-chart')).toBeVisible();
  await expect(page.getByTestId('volume-interval')).toBeVisible();
  await expect(page.getByTestId('leaderboard-row').first()).toBeVisible();
  await expect(page.getByTestId('analytics-trade-row').first()).toBeVisible();
  await expect(page.getByTestId('pnl-distribution')).toBeVisible();

  await page.getByTestId('nav-trade').click();
  await expect(page.getByTestId('market-mark')).toHaveText(/\d/);
  await expect(page.getByTestId('analytics-page')).toHaveCount(0);
  expect(new URL(page.url()).search).toContain('api=');
});
