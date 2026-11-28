import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './e2e',
  testMatch: '*.e2e.ts', // not .spec.ts: vitest would pick that up
  outputDir: 'node_modules/.playwright',
  workers: 1, // one server and one database for every test
  use: { baseURL: 'http://127.0.0.1:18080', trace: 'off' },
  webServer: { command: 'node e2e/serve.mjs', url: 'http://127.0.0.1:18080/', timeout: 600_000, reuseExistingServer: false },
})
