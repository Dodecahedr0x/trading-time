import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env.TT_WEB_PORT ?? 14173);

export default defineConfig({
  testDir: './tests',
  timeout: 180_000,
  expect: { timeout: 60_000 },
  workers: 1,
  reporter: [['list']],
  use: { baseURL: `http://127.0.0.1:${port}`, trace: 'retain-on-failure', ...devices['Desktop Chrome'] },
  webServer: { command: 'node serve.mjs', url: `http://127.0.0.1:${port}`, reuseExistingServer: true, timeout: 600_000 },
});
