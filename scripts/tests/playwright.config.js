// @ts-check
const { defineConfig, devices } = require('@playwright/test');

/**
 * Playwright configuration for Auralis Desktop E2E testing.
 * Strictly configured outside `src/`, `ui/`, and `gen/android/`.
 */
module.exports = defineConfig({
  testDir: './',
  testMatch: /(.*_test\.js|.*_diagnostics\.js)/,
  timeout: 60000,
  expect: {
    timeout: 10000,
  },
  fullyParallel: false,
  workers: 1,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: 'http://localhost:1420',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [
    {
      name: 'Desktop WebKit',
      use: { ...devices['Desktop Safari'] },
    },
    {
      name: 'Desktop Chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
