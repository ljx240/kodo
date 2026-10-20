"use strict";

const assert = require("assert");
const { report, KINDS } = require("../src/service");
const { summarizeByCustomer } = require("../src/report");

// The kind the dashboard calls, registered in KINDS.
assert.ok(KINDS.includes("by-customer"), "by-customer must be a known kind");
assert.deepStrictEqual(report("by-customer"), summarizeByCustomer());
assert.deepStrictEqual(
  report("by-customer").map((row) => row.name),
  ["Cy", "Bo", "Ada"]
);

// daily keeps its behaviour, unknown kinds stay rejected.
const daily = report("daily");
assert.strictEqual(daily.length, 6);
assert.deepStrictEqual(daily[0], { day: "2026-09-01", totalCents: 4500 });
assert.deepStrictEqual(daily[2], { day: "2026-09-03", totalCents: 0 }, "cancelled day");
assert.throws(() => report("hourly"), /unknown report kind/);

console.log("service.test ok");
