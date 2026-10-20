"use strict";

const NAMES = { "c-1": "Ada", "c-2": "Bo", "c-3": "Cy" };

function findName(customerId) {
  return NAMES[customerId] || `unknown:${customerId}`;
}

function known(customerId) {
  return Boolean(NAMES[customerId]);
}

module.exports = { findName, known };
