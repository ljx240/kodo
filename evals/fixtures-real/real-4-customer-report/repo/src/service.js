"use strict";

const { dailyTotals } = require("./report");

// The dashboard calls report(kind); every kind it names must be handled here.
function report(kind) {
  if (kind === "daily") return dailyTotals();
  throw new Error(`unknown report kind: ${kind}`);
}

module.exports = { report, KINDS: ["daily"] };
