// Programmatic UI audit at the reference viewport: geometry, typography,
// overflow, structure. Prints JSON findings per demo route.
import { chromium } from "@playwright/test";

const base = "http://localhost:1420";
const ROUTES = [
  ["conversation", "/ui-demo/conversation"],
  ["conversation-closed", "/ui-demo/conversation?inspector=closed"],
  ["trace", "/ui-demo/trace"],
  ["archive", "/ui-demo/archive"],
  ["settings", "/ui-demo/settings"],
];

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1586, height: 992 },
  deviceScaleFactor: 1,
  reducedMotion: "reduce",
  colorScheme: "light",
});

const report = {};

for (const [name, url] of ROUTES) {
  await page.goto(base + url);
  await page.waitForLoadState("networkidle");
  await page.waitForTimeout(300);

  report[name] = await page.evaluate(() => {
    const box = (sel) => {
      const el = document.querySelector(sel);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
    };
    const style = (sel, prop) => {
      const el = document.querySelector(sel);
      return el ? getComputedStyle(el)[prop] : null;
    };

    // Horizontal overflow candidates (text clipped without ellipsis intent).
    const overflow = [];
    for (const el of document.querySelectorAll("*")) {
      if (el.scrollWidth > el.clientWidth + 2 && el.clientWidth > 0) {
        const s = getComputedStyle(el);
        const clipped = s.textOverflow === "ellipsis" || s.overflowX === "auto" || s.overflowX === "scroll";
        if (!clipped) {
          overflow.push({
            cls: String(el.className).slice(0, 60),
            tag: el.tagName,
            sw: el.scrollWidth,
            cw: el.clientWidth,
            text: (el.textContent || "").slice(0, 60),
          });
        }
      }
    }

    // Visible trace rows (conversation density check).
    const traceRows = [...document.querySelectorAll(".trace-row")].filter((row) => {
      const r = row.getBoundingClientRect();
      return r.top >= 0 && r.bottom <= window.innerHeight && r.height > 0;
    });

    // Font sizes of key landmarks.
    const fonts = {};
    for (const [k, sel] of Object.entries({
      pageTitle: "h1, .page-title, .topbar-title",
      navItem: ".nav-item",
      body: ".msg-text, .answer-body, .final-answer",
      meta: ".tree-conversation-time, .msg-time",
      sectionTitle: ".ins-section-title",
    })) {
      const el = document.querySelector(sel);
      if (el) fonts[k] = getComputedStyle(el).fontSize;
    }

    // Structure checks.
    const sidebarText = document.querySelector(".sidebar")?.innerText ?? "";
    const navLabels = [...document.querySelectorAll(".sidebar .nav-item")].map((n) => n.textContent?.trim());

    return {
      viewport: { w: window.innerWidth, h: window.innerHeight },
      docScrollW: document.documentElement.scrollWidth,
      sidebar: box(".sidebar"),
      topbar: box(".topbar"),
      inspector: box(".inspector"),
      rail: box(".ins-rail"),
      panel: box(".ins-panel"),
      main: box(".main"),
      composer: box(".composer"),
      fonts,
      navLabels,
      hasAccountRow: /退出|sign.?out|avatar|账户|li jianxue/i.test(sidebarText),
      visibleTraceRows: traceRows.length,
      overflow: overflow.slice(0, 12),
      // Page-specific structure
      archiveTable: !!document.querySelector("table, .archive-table, .table"),
      settingsCats: [...document.querySelectorAll(".settings-nav button, .settings-nav a")].map((n) =>
        n.textContent?.trim(),
      ),
      traceTabs: [...document.querySelectorAll(".trace-tabs button, .tabs button, [role=tab]")].map((n) =>
        n.textContent?.trim(),
      ),
      bodyHead: (document.body.innerText || "").slice(0, 100).replace(/\s+/g, " "),
    };
  });
}

// Settings: click every category, ensure the pane is non-empty.
await page.goto(base + "/ui-demo/settings");
await page.waitForLoadState("networkidle");
const cats = await page.locator(".settings-nav button, .settings-nav a").all();
report.settingsCategoryPanes = [];
for (const cat of cats) {
  const label = (await cat.textContent())?.trim();
  await cat.click();
  await page.waitForTimeout(120);
  const pane = await page.evaluate(() => {
    const el = document.querySelector(".settings-detail, .settings-section, .settings-pane");
    return el ? el.innerText.trim().slice(0, 80) : null;
  });
  report.settingsCategoryPanes.push({ label, empty: !pane || pane.length < 5, pane });
}

console.log(JSON.stringify(report, null, 2));
await browser.close();
