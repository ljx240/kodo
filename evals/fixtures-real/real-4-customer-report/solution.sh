#!/bin/sh
# Reviewer's canonical fix: implement summarizeByCustomer per docs/report-spec.md
# and register the by-customer kind in src/service.js. Applied to a throwaway
# copy by the baselines; never copied into the model's work dir.
set -e
cd "$(dirname "$0")/repo"

node -e '
const fs = require("fs");

const report = "src/report.js";
const r = fs.readFileSync(report, "utf8");
if (!r.includes(`const { groupBy } = require("./format");`)) throw new Error("report.js anchor moved");

const body = `function summarizeByCustomer(orders) {
  const groups = groupBy(orders || listOrders(), (order) => order.customerId);
  const rows = [];
  for (const [customerId, bucket] of groups) {
    const paid = bucket.filter((order) => order.status === "paid");
    if (paid.length === 0) continue;
    rows.push({
      customerId,
      name: findName(customerId),
      totalCents: paid.reduce((sum, order) => sum + order.totalCents, 0),
      count: paid.length,
    });
  }
  return rows.sort(
    (a, b) => b.totalCents - a.totalCents || (a.customerId < b.customerId ? -1 : 1)
  );
}
`;
const stub = `function summarizeByCustomer(orders) {
  throw new Error("summarizeByCustomer is not implemented");
}
`;
if (!r.includes(stub)) throw new Error("report.js stub moved");

fs.writeFileSync(report, r
  .replace(`const { groupBy } = require("./format");`,
    `const { groupBy } = require("./format");\nconst { findName } = require("./customers");`)
  .replace(stub, body));

const service = "src/service.js";
const s = fs.readFileSync(service, "utf8");
if (!s.includes(`const { dailyTotals } = require("./report");`)) throw new Error("service.js anchor moved");
fs.writeFileSync(service, s
  .replace(`const { dailyTotals } = require("./report");`,
    `const { dailyTotals, summarizeByCustomer } = require("./report");`)
  .replace(`  if (kind === "daily") return dailyTotals();`,
    `  if (kind === "daily") return dailyTotals();\n  if (kind === "by-customer") return summarizeByCustomer();`)
  .replace(`module.exports = { report, KINDS: ["daily"] };`,
    `module.exports = { report, KINDS: ["daily", "by-customer"] };`));
'
