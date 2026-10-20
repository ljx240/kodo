"use strict";

const { roundCents } = require("./money");

// Tier table agreed with finance on 2026-09-02:
//   subtotal >= 100.00  -> 10 %
//   subtotal >=  50.00  ->  5 %
//   otherwise           ->  0 %
const GOLD_CENTS = 10000;
const SILVER_CENTS = 5000;

function discountRate(subtotalCents) {
  if (subtotalCents > GOLD_CENTS) return 0.1;
  if (subtotalCents >= SILVER_CENTS) return 0.05;
  return 0;
}

function discountFor(subtotalCents) {
  return roundCents(subtotalCents * discountRate(subtotalCents));
}

module.exports = { discountRate, discountFor, GOLD_CENTS, SILVER_CENTS };
