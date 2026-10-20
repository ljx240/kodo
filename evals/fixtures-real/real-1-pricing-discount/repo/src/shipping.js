"use strict";

// Shipping is flat per band; the bands come from the 2026 carrier contract.
function shippingFor(weightGrams) {
  if (weightGrams <= 0) return 0;
  if (weightGrams <= 500) return 350;
  if (weightGrams <= 2000) return 750;
  return 1200;
}

module.exports = { shippingFor };
