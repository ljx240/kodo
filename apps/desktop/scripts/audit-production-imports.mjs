#!/usr/bin/env node
/**
 * Production-route import audit.
 *
 * Conversation / Trace / Settings / Inspector must not import demo or fixture
 * data modules. Demo content reaches those surfaces only as props from the
 * shell (src/data/demoState.ts is the sole boundary that loads the fixture).
 *
 * Usage: node scripts/audit-production-imports.mjs
 * Exit 1 on any violation.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Files under these trees are production routes / shared UI. */
const PRODUCTION_GLOBS = [
  "src/pages",
  "src/conversation",
  "src/inspector",
];

/** Forbidden module specifiers (relative import sources). */
const FORBIDDEN = [
  /from\s+["'].*\/data\/fixture["']/,
  /from\s+["'].*\/data\/demo["']/,
  /from\s+["']\.\.?\/data\/fixture["']/,
  /from\s+["']\.\.?\/data\/demo["']/,
  /import\s*\(\s*["'].*\/data\/(fixture|demo)["']/,
  /require\s*\(\s*["'].*\/data\/(fixture|demo)["']/,
];

/** demoState is the boundary — allowed only outside these trees. */
function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (/\.(ts|tsx)$/.test(name)) out.push(full);
  }
  return out;
}

const violations = [];
for (const sub of PRODUCTION_GLOBS) {
  const dir = join(root, sub);
  let files;
  try {
    files = walk(dir);
  } catch {
    continue;
  }
  for (const file of files) {
    const source = readFileSync(file, "utf8");
    for (const pattern of FORBIDDEN) {
      if (pattern.test(source)) {
        violations.push(`${relative(root, file)} matches ${pattern}`);
      }
    }
  }
}

if (violations.length > 0) {
  console.error("Production-route import audit FAILED:");
  for (const line of violations) console.error(`  - ${line}`);
  process.exit(1);
}

console.log("Production-route import audit OK (pages/conversation/inspector free of demo/fixture imports).");
