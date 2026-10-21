import { test, expect } from '@playwright/test';
import { loadEnv } from '../wallet.js';

const env = loadEnv();
const appUrl = `/?rpc=${encodeURIComponent(env.rpc_url)}&api=${encodeURIComponent(env.api_url)}`;

test('docs page explains the product and deep-links to sections', async ({ page }) => {
  await page.goto(appUrl);
  await page.getByTestId('nav-docs').click();
  await expect(page.getByTestId('docs-page')).toBeVisible();
  await expect(page).toHaveURL(/#\/docs$/);
  const sections = page.getByTestId('docs-section');
  expect(await sections.count()).toBeGreaterThanOrEqual(10);
  await expect(page.getByRole('heading', { name: 'Funding' })).toBeVisible();
  await expect(page.getByTestId('docs-page')).toContainText('5 bp');

  await page.getByTestId('docs-toc').getByRole('link', { name: 'Margin and liquidation' }).click();
  await expect(page).toHaveURL(/#\/docs\/margin$/);
  await expect(page.locator('#doc-margin')).toBeInViewport();

  await page.goto(`${appUrl}#/docs/funding`);
  await expect(page.locator('#doc-funding')).toBeInViewport();
  await page.getByTestId('nav-trade').click();
  await expect(page.getByTestId('market-mark')).toHaveText(/\d/);
});
