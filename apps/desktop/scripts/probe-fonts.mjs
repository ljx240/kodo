import { chromium } from "@playwright/test";
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: 1586, height: 992 } });
await p.goto("http://localhost:1420/ui-demo/conversation");
await p.waitForLoadState("networkidle");
await p.waitForTimeout(300);
const out = await p.evaluate(() => {
  const pick = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const s = getComputedStyle(el);
    return { sel, fontSize: s.fontSize, color: s.color, text: (el.textContent||"").slice(0,50) };
  };
  const leaves = [];
  for (const el of document.querySelectorAll(".final *")) {
    if (el.children.length === 0 && (el.textContent||"").trim()) {
      const s = getComputedStyle(el);
      leaves.push({ cls: String(el.className).slice(0,40), tag: el.tagName, fs: s.fontSize, text: (el.textContent||"").trim().slice(0,40) });
    }
  }
  const composer = document.querySelector(".composer")?.getBoundingClientRect();
  const box = document.querySelector(".composer-box")?.getBoundingClientRect();
  const rows = [...document.querySelectorAll(".tree-project")].map(r=>Math.round(r.getBoundingClientRect().height));
  const convs = [...document.querySelectorAll(".tree-conversation")].map(r=>Math.round(r.getBoundingClientRect().height));
  const trace = [...document.querySelectorAll(".trace-row")].slice(0,5).map(r=>Math.round(r.getBoundingClientRect().height));
  return {
    finalLeaves: leaves.slice(0, 40),
    axes: pick(".answer-axes"),
    composerH: composer && Math.round(composer.height),
    boxH: box && Math.round(box.height),
    projRows: rows.slice(0,4),
    convRows: convs.slice(0,4),
    traceRows: trace,
    traceOutput: pick(".trace-output"),
  };
});
console.log(JSON.stringify(out, null, 2));
await b.close();
