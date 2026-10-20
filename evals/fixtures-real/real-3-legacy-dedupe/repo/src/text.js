"use strict";

// Shared text helpers. Document formatting must live here, not inside the
// legacy renderers — the two copies in src/legacy are the bug this task is
// about (see docs/adr-004.md).
const MONTHS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

function titleCase(text) {
  return String(text)
    .split(" ")
    .filter(Boolean)
    .map((word) => word[0].toUpperCase() + word.slice(1).toLowerCase())
    .join(" ");
}

function collapseSpaces(text) {
  return String(text).replace(/\s+/g, " ").trim();
}

module.exports = { MONTHS, titleCase, collapseSpaces };
