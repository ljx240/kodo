// Targeted post-fix verification: archive columns, settings search, composer row.
import { chromium } from "@playwright/test";
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1586, height: 992 }, reducedMotion: "reduce" });

// Archive
await p.goto("http://localhost:1420/ui-demo/archive");
await p.waitForLoadState("networkidle");
const arc = await p.evaluate(() => {
  const ths = [...document.querySelectorAll(".archive-table th")].map((th) => ({
    text: th.textContent.trim(),
    w: Math.round(th.getBoundingClientRect().width),
    overflow: getComputedStyle(th).overflow,
  }));
  const td1 = document.querySelector(".archive-table tbody td");
  const title = document.querySelector(".arc-title-text");
  const tr = [...document.querySelectorAll(".archive-table tbody tr")][0];
  const row = tr ? [...tr.children].map((td) => Math.round(td.getBoundingClientRect().width)) : [];
  const foot = document.querySelector(".table-foot")?.innerText.replace(/\s+/g, " ");
  return { ths, row, titleClips: title ? title.scrollWidth > title.clientWidth : null, foot };
});

// Settings search
await p.goto("http://localhost:1420/ui-demo/settings");
await p.waitForLoadState("networkidle");
const head = await p.evaluate(() => {
  const f = document.querySelector(".page-head .search-field");
  return f ? { placeholder: f.querySelector("input")?.placeholder, kbd: f.querySelector("kbd")?.textContent } : null;
});
// Type a query and check nav filters
await p.fill(".page-head .search-field input", "appear");
await p.waitForTimeout(150);
const navAfter = await p.locator(".settings-nav-item").allTextContents();
const detailAfter = await p.locator(".settings-detail h2").textContent();
// No-match query
await p.fill(".page-head .search-field input", "zzzz");
await p.waitForTimeout(150);
const navNone = await p.locator(".settings-nav-item").count();
const emptyNote = await p.locator(".settings-detail .empty-note").textContent().catch(() => null);
// Clear + ⌘F focus check
await p.fill(".page-head .search-field input", "");
await p.waitForTimeout(100);
await p.locator(".settings-detail h2").click().catch(() => {});
await p.keyboard.press("Meta+f");
const focused = await p.evaluate(() => document.activeElement?.getAttribute("aria-label"));

// Composer structure
await p.goto("http://localhost:1420/ui-demo/conversation");
await p.waitForLoadState("networkidle");
const comp = await p.evaluate(() => {
  const row = document.querySelector(".composer-row");
  const kids = row ? [...row.children].map((c) => c.className.split(" ")[0]) : [];
  const r = row?.getBoundingClientRect();
  const send = document.querySelector(".composer-send")?.getBoundingClientRect();
  const input = document.querySelector(".composer-input")?.getBoundingClientRect();
  const plus = document.querySelector(".composer-plus")?.getBoundingClientRect();
  const box = document.querySelector(".composer-box")?.getBoundingClientRect();
  const composer = document.querySelector(".composer")?.getBoundingClientRect();
  return {
    kids,
    rowH: r && Math.round(r.height),
    sameLine: input && send && Math.abs(input.y - send.y) < 40 && send.y >= input.y - 5,
    plusLeftOfInput: plus && input && plus.x < input.x,
    inputW: input && Math.round(input.width),
    boxH: box && Math.round(box.height),
    composerH: composer && Math.round(composer.height),
    bottomGap: box && Math.round(992 - box.bottom),
  };
});

console.log(JSON.stringify({ arc, head, navAfter, detailAfter, navNone, emptyNote, focused, comp }, null, 2));
await b.close();
