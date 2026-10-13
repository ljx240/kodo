// Captures the welcome stage at multiple viewports. Loads the live
// conversation route (no demo), so activeConversationId stays null and
// the welcome stage renders.
import { chromium } from "@playwright/test";
import { mkdir } from "node:fs/promises";

const outDir = process.argv[2] ?? "/tmp/kodo-ui-audit";

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: 1586, height: 992 },
  deviceScaleFactor: 1,
  reducedMotion: "reduce",
  colorScheme: "light",
});
const page = await ctx.newPage();

// Live route (no /ui-demo prefix) → activeConversationId starts null → welcome renders.
await page.goto("http://localhost:1420/conversation?inspector=open");
await page.waitForLoadState("networkidle");
await page.waitForTimeout(400);

await page.screenshot({ path: `${outDir}/06-welcome-stage.png` });
console.log("captured welcome stage 1586");

await page.setViewportSize({ width: 1440, height: 900 });
await page.waitForTimeout(200);
await page.screenshot({ path: `${outDir}/07-welcome-stage-1440.png` });
console.log("captured welcome stage 1440");

await page.setViewportSize({ width: 1280, height: 800 });
await page.waitForTimeout(200);
await page.screenshot({ path: `${outDir}/08-welcome-stage-1280.png` });
console.log("captured welcome stage 1280");

// Welcome stage with inspector closed
await page.setViewportSize({ width: 1586, height: 992 });
await page.goto("http://localhost:1420/conversation?inspector=closed");
await page.waitForLoadState("networkidle");
await page.waitForTimeout(400);
await page.screenshot({ path: `${outDir}/09-welcome-no-inspector.png` });
console.log("captured welcome stage no inspector");

await browser.close();
console.log("done ->", outDir);