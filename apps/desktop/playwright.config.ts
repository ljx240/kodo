import { defineConfig } from "@playwright/test";

/** Reference viewport from docs/design/UI_ACCEPTANCE.md. */
const VIEWPORT = { width: 1586, height: 992 };

export default defineConfig({
  testDir: "./tests/visual",
  fullyParallel: true,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:1420",
    viewport: VIEWPORT,
    deviceScaleFactor: 1,
    reducedMotion: "reduce",
    colorScheme: "light",
  },
  expect: {
    toHaveScreenshot: {
      // The demo state is frozen, so the render is byte-identical run to run;
      // this leaves slack for font antialiasing only, not for geometry.
      maxDiffPixelRatio: 0.001,
      animations: "disabled",
    },
  },
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
