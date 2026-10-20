"use strict";

const { roundCents } = require("./money");

const DEFAULT_RATE = 0.08;

function salesTax(netCents, rate) {
  const applied = rate === undefined ? DEFAULT_RATE : rate;
  return roundCents(netCents * applied);
}

module.exports = { salesTax, DEFAULT_RATE };
