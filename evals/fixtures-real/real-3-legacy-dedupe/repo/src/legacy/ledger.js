"use strict";

// Ledger lines are already single-sourced; nothing to change here.
function summarize(entries) {
  const totals = {};
  for (const entry of entries) {
    totals[entry.account] = (totals[entry.account] || 0) + entry.amountCents;
  }
  return Object.keys(totals)
    .sort()
    .map((account) => `${account}: ${totals[account]}`)
    .join("\n");
}

module.exports = { summarize };
