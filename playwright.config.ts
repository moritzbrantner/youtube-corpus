import { defineConfig, devices } from "@playwright/test";

// Browser tests run the Vite frontend against route-mocked `/api` fixtures; no Postgres needed.
export default defineConfig({
  testDir: "tests/e2e",
  workers: 2,
  expect: { timeout: 10_000 },
  use: {
    baseURL: "http://127.0.0.1:41871",
    trace: "on-first-retry",
  },
  webServer: {
    command: "bun run dev:frontend -- --port 41871 --strictPort",
    url: "http://127.0.0.1:41871",
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
  },
  projects: [
    { name: "desktop-chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "mobile-chromium", use: { ...devices["Pixel 5"] } },
  ],
});
