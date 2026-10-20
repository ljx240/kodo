"use strict";

const { listOrders } = require("./orders");
const { groupBy } = require("./format");

// Revenue per placement day, paid orders only.
function dailyTotals(orders) {
  const rows = [];
  const groups = groupBy(orders || listOrders(), (order) => order.placedAt);
  for (const [day, bucket] of groups) {
    const totalCents = bucket
      .filter((order) => order.status === "paid")
      .reduce((sum, order) => sum + order.totalCents, 0);
    rows.push({ day, totalCents });
  }
  return rows.sort((a, b) => (a.day < b.day ? -1 : 1));
}

// TODO(2026-09-18): the customer-facing report the sales team asked for.
// See docs/report-spec.md for the exact shape, then wire it into src/service.js.
function summarizeByCustomer(orders) {
  throw new Error("summarizeByCustomer is not implemented");
}

module.exports = { dailyTotals, summarizeByCustomer };
