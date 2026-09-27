import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  use: { baseURL: 'http://127.0.0.1:5099', headless: true },
  webServer: {
    command: '../scripts/web-test-server.sh',
    url: 'http://127.0.0.1:5099/api/health',
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
