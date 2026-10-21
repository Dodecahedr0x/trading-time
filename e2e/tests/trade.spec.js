import { test, expect } from '@playwright/test';
import { installWallet, loadEnv } from '../wallet.js';

const env = loadEnv();
const appUrl = `/?rpc=${encodeURIComponent(env.rpc_url)}&api=${encodeURIComponent(env.api_url)}`;

test.beforeEach(async ({ page }) => {
  page.on('console', (m) => { if (m.type() === 'error') console.log('[page]', m.text()); });
  const address = await installWallet(page, env);
  await page.goto(appUrl);
  await expect(page.getByTestId('market-mark')).toHaveText(/\d/);
  await expect(page.getByTestId('quotes-ask-row').first()).toBeVisible();
  await page.getByTestId('connect-wallet').click();
  await expect(page.getByTestId('wallet-address')).toHaveAttribute('data-address', address);
});

test('market view explains the product', async ({ page }) => {
  await expect(page.getByTestId('market-index')).toHaveText(/\d/);
  await expect(page.getByTestId('sol-price')).toHaveText(/^\$\d/);
  await expect(page.getByTestId('explain-line')).not.toBeEmpty();
  await expect(page.getByTestId('chart')).toBeVisible();
  await expect(page.getByTestId('quotes-bid-row').first()).toBeVisible();
});

test('buy long time at market opens a position', async ({ page }) => {
  await page.getByTestId('order-type-market').click();
  await page.getByTestId('side-long').click();
  await page.getByTestId('size-input').fill('250');
  await expect(page.getByTestId('you-are-buying')).toContainText(/long time/i);
  await page.getByTestId('buy-button').click();
  await expect(page.getByTestId('tx-status')).toHaveText('Confirmed');
  await page.getByTestId('tab-positions').click();
  await expect(page.getByTestId('positions-row')).toHaveCount(1);
  await expect(page.getByTestId('positions-row').first()).toContainText(/long/i);
});

test('place and cancel a resting limit order', async ({ page }) => {
  await page.getByTestId('order-type-limit').click();
  await page.getByTestId('side-short').click();
  // Short time fills when the price rises to the limit; a far limit keeps the order resting.
  await page.getByTestId('limit-input').fill('100d');
  await page.getByTestId('size-input').fill('100');
  await page.getByTestId('buy-button').click();
  await expect(page.getByTestId('tx-status')).toHaveText('Confirmed');
  await page.getByTestId('tab-orders').click();
  const rows = page.getByTestId('orders-row');
  await expect(rows).toHaveCount(1);
  await rows.first().getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByTestId('tx-status')).toHaveText('Confirmed');
  await expect(rows).toHaveCount(0);
});
