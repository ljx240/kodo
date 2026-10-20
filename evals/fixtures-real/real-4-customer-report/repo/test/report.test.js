"use strict";

// docs/report-spec.md is the contract this suite encodes.
const assert = require("assert");
const { summarizeByCustomer, dailyTotals } = require("../src/report");

assert.deepStrictEqual(summarizeByCustomer(), [
  { customerId: "c-3", name: "Cy", totalCents: 5400, count: 1 },
  { customerId: "c-2", name: "Bo", totalCents: 4500, count: 1 },
  { customerId: "c-1", name: "Ada", totalCents: 2100, count: 2 },
]);

assert.deepStrictEqual(summarizeByCustomer([]), [], "empty input gives an empty list");
assert.deepStrictEqual(
  summarizeByCustomer([
    { customerId: "c-9", totalCents: 100, status: "paid" },
    { customerId: "c-9", totalCents: 50, status: "cancelled" },
  ]),
  [{ customerId: "c-9", name: "unknown:c-9", totalCents: 100, count: 1 }],
  "cancelled rows count for neither total nor count"
);
assert.deepStrictEqual(
  summarizeByCustomer([{ customerId: "c-1", totalCents: 100, status: "refunded" }]),
  [],
  "a customer with no paid order does not appear"
);
// Tie-break: same total, customerId ascending.
assert.deepStrictEqual(
  summarizeByCustomer([
    { customerId: "c-b", totalCents: 700, status: "paid" },
    { customerId: "c-a", totalCents: 700, status: "paid" },
  ]),
  [
    { customerId: "c-a", name: "unknown:c-a", totalCents: 700, count: 1 },
    { customerId: "c-b", name: "unknown:c-b", totalCents: 700, count: 1 },
  ]
);
// Daily totals stay put.
assert.deepStrictEqual(dailyTotals().length, 6);

console.log("report.test ok");
