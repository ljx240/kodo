// Pixel-measures reference PNGs and current screenshots: vertical panel edges,
// horizontal rule positions (row pitch), composer box bounds. No vision needed.
import { chromium } from "@playwright/test";
import { readFile } from "node:fs/promises";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: node measure-reference.mjs <png...>");
  process.exit(1);
}

const browser = await chromium.launch();
const page = await browser.newPage();

for (const file of files) {
  const b64 = (await readFile(file)).toString("base64");
  const result = await page.evaluate(async (dataUrl) => {
    const img = new Image();
    img.src = dataUrl;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext("2d");
    ctx.drawImage(img, 0, 0);
    const { data, width, height } = ctx.getImageData(0, 0, c.width, c.height);

    const at = (x, y) => {
      const i = (y * width + x) * 4;
      return [data[i], data[i + 1], data[i + 2]];
    };
    const near = (rgb, target, tol = 10) =>
      Math.abs(rgb[0] - target[0]) <= tol &&
      Math.abs(rgb[1] - target[1]) <= tol &&
      Math.abs(rgb[2] - target[2]) <= tol;

    // Vertical edges: columns where the colour changes and stays changed.
    // Report background bands along row y=height/2 as panel boundaries.
    const midY = Math.floor(height / 2);
    const bands = [];
    let start = 0;
    let cur = at(0, midY).join(",");
    for (let x = 1; x < width; x++) {
      const c2 = at(x, midY).join(",");
      // Treat adjacent near-identical colours as one band (quantise to 8).
      const q = (s) => s.split(",").map((v) => Math.round(+v / 8)).join(",");
      if (q(c2) !== q(cur)) {
        if (x - start >= 8) bands.push({ x0: start, x1: x, rgb: cur.split(",").map(Number) });
        start = x;
        cur = c2;
      }
    }
    bands.push({ x0: start, x1: width, rgb: cur.split(",").map(Number) });

    // Horizontal rules: rows where >=60% of x in [x0,x1] is border-ish grey.
    const rules = (x0, x1) => {
      const out = [];
      for (let y = 0; y < height; y++) {
        let hit = 0;
        const span = x1 - x0;
        for (let x = x0; x < x1; x += 2) {
          const [r, g, b] = at(x, y);
          if (r >= 215 && r <= 240 && g >= 218 && g <= 243 && b >= 225 && b <= 248 && Math.abs(r - b) <= 20) hit++;
        }
        if (hit / (span / 2) >= 0.6) out.push(y);
      }
      // collapse consecutive
      const merged = [];
      for (const y of out) {
        if (merged.length && y - merged[merged.length - 1] <= 2) continue;
        merged.push(y);
      }
      return merged;
    };

    return { width, height, bands, midRules: rules(Math.floor(width * 0.2), Math.floor(width * 0.7)) };
  }, `data:image/png;base64,${b64}`);

  const bands = result.bands
    .filter((b) => b.x1 - b.x0 >= 8)
    .map((b) => `${b.x0}-${b.x1}(${b.x1 - b.x0})rgb(${b.rgb.join(",")})`);
  const diffs = [];
  for (let i = 1; i < result.midRules.length; i++) diffs.push(result.midRules[i] - result.midRules[i - 1]);
  console.log(`\n== ${file} (${result.width}x${result.height})`);
  console.log("bands@mid:", bands.slice(0, 14).join(" | "));
  console.log("rules:", result.midRules.slice(0, 30).join(","));
  console.log("pitches:", diffs.slice(0, 25).join(","));
}

await browser.close();
