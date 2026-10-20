"use strict";

// All money in this project is integer cents. Never use floats for a total.
function toCents(amount) {
  return Math.round(amount * 100);
}

function roundCents(value) {
  return Math.round(value);
}

function formatCents(cents) {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  return `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
}

module.exports = { toCents, roundCents, formatCents };
