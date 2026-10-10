// Captures UI_ACCEPTANCE demo routes at the reference viewport for visual review.
// Usage: node scripts/capture-ui.mjs [outDir]
import { chromium } from "@playwright/test";
import { mkdir } from "node:fs/promises";

const outDir = process.argv[2] ?? "/tmp/kodo-ui-audit";
const base = "http://localhost:1420";

const shots = [
  { name: "01-conversation-main", url: "/ui-demo/conversation" },
  { name: "02-conversation-inspector-closed", url: "/ui-demo/conversation?inspector=closed" },
  { name: "03-response-trace", url: "/ui-demo/trace" },
  { name: "04-archive", url: "/ui-demo/archive" },
  { name: "05-settings", url: "/ui-demo/settings" },
];

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1586, height: 992 },
  deviceScaleFactor: 1,
  reducedMotion: "reduce",
  colorScheme: "light",
});

for (const { name, url } of shots) {
  await page.goto(base + url);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${outDir}/${name}.png` });
  console.log("captured", name);
}

await browser.close();
console.log("done ->", outDir);
