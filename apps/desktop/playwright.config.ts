import { defineConfig } from "@playwright/test";

const localBrowser = process.env.KODO_PLAYWRIGHT_EXECUTABLE_PATH;

/** Reference viewport from docs/design/UI_ACCEPTANCE.md. */
const VIEWPORT = { width: 1586, height: 992 };

/**
 * Two projects so CI can split stages cleanly:
 *
 * - `behaviour` — interaction / a11y / layout asserts (no pixel snapshots).
 *   Runs on Linux; independent of platform fonts.
 * - `visual`    — `toHaveScreenshot` against `*-darwin.png` baselines.
 *   Runs only on macOS runners so fonts match the committed baselines.
 *   CI never passes `--update-snapshots`.
 */
export default defineConfig({
  testDir: "./tests/visual",
  fullyParallel: true,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: "http://localhost:1420",
    viewport: VIEWPORT,
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
    colorScheme: "light",
    ...(localBrowser ? { launchOptions: { executablePath: localBrowser } } : {}),
  },
  expect: {
    toHaveScreenshot: {
      // The demo state is frozen, so the render is byte-identical run to run;
      // this leaves slack for font antialiasing only, not for geometry.
      maxDiffPixelRatio: 0.001,
      animations: "disabled",
    },
  },
  projects: [
    {
      name: "behaviour",
      testIgnore: /pages\.spec\.ts/,
    },
    {
      name: "visual",
      testMatch: /pages\.spec\.ts/,
    },
  ],
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
