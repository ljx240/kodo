#!/usr/bin/env node
/**
 * WCAG contrast audit for Kodo design tokens.
 *
 * Readable tokens must reach AA (4.5:1) against the surfaces they sit on.
 * Decorative tokens (muted icons, chevrons) are reported but exempt.
 *
 * Usage: node scripts/contrast-audit.mjs
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tokensPath = join(root, "src/styles/tokens.css");

function parseTokens(css) {
  const map = new Map();
  const rootBlock = css.match(/:root\s*\{([^}]+)\}/);
  if (!rootBlock) throw new Error("tokens.css: no :root block");
  for (const line of rootBlock[1].split(";")) {
    const m = line.match(/(--kodo-[a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{3,8})\b/);
    if (m) map.set(m[1], m[2]);
  }
  return map;
}

function hexToRgb(hex) {
  let h = hex.slice(1);
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
}

function channel(value) {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const [r, g, b] = hexToRgb(hex).map(channel);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  const [hi, lo] = la >= lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

const BACKGROUNDS = [
  "--kodo-canvas",
  "--kodo-sidebar",
  "--kodo-surface",
  "--kodo-hover",
  "--kodo-selected",
];

/** Informational text colors that must meet WCAG AA on every surface. */
const READABLE = [
  "--kodo-text",
  "--kodo-text-secondary",
  "--kodo-accent",
  "--kodo-success-text",
  "--kodo-warning-text",
  "--kodo-error",
];

/** Inverse text on solid accent controls. */
const INVERSE = [["#ffffff", "--kodo-accent"], ["#ffffff", "--kodo-accent-hover"], ["#ffffff", "--kodo-text"]];

/** Decorative only — icons, chevrons, separators. Never informational body text. */
const DECORATIVE = ["--kodo-text-muted"];

const AA = 4.5;
const tokens = parseTokens(readFileSync(tokensPath, "utf8"));

let failures = 0;
const lines = [];

function report(ok, label, ratio, required) {
  const status = ok ? "PASS" : "FAIL";
  if (!ok) failures += 1;
  lines.push(`${status}  ${ratio.toFixed(2)}:1  (need ${required})  ${label}`);
}

lines.push("Kodo color contrast audit — WCAG AA");
lines.push("");

lines.push("Readable text on surfaces:");
for (const fg of READABLE) {
  const color = tokens.get(fg);
  if (!color) {
    report(false, `${fg} missing from tokens`, 0, AA);
    continue;
  }
  for (const bg of BACKGROUNDS) {
    const bgc = tokens.get(bg);
    if (!bgc) {
      report(false, `${bg} missing from tokens`, 0, AA);
      continue;
    }
    const ratio = contrast(color, bgc);
    report(ratio >= AA, `${fg} ${color} on ${bg} ${bgc}`, ratio, AA);
  }
}

lines.push("");
lines.push("Inverse text on solid controls:");
for (const [fg, bg] of INVERSE) {
  const bgc = tokens.get(bg) ?? bg;
  const ratio = contrast(fg, bgc);
  report(ratio >= AA, `#ffffff on ${bg} ${bgc}`, ratio, AA);
}

lines.push("");
lines.push("Decorative (exempt — must not carry informational text):");
for (const fg of DECORATIVE) {
  const color = tokens.get(fg);
  if (!color) {
    report(false, `${fg} missing`, 0, 0);
    continue;
  }
  const onCanvas = contrast(color, tokens.get("--kodo-canvas"));
  lines.push(`INFO  ${onCanvas.toFixed(2)}:1  ${fg} ${color} on canvas (decorative only)`);
}

lines.push("");
lines.push(failures === 0 ? "All readable pairs meet WCAG AA." : `${failures} pair(s) failed WCAG AA.`);
console.log(lines.join("\n"));
process.exit(failures === 0 ? 0 : 1);
