"use strict";

const assert = require("assert");
const { createCart, addItem } = require("../src/cart");
const { finalizeOrder } = require("../src/orders");
const { discountRate } = require("../src/discounts");

function cartOf(rows) {
  const cart = createCart();
  for (const row of rows) addItem(cart, row);
  return cart;
}

// Tier boundaries are inclusive: exactly 100.00 is a gold-tier order.
assert.strictEqual(discountRate(10000), 0.1, "100.00 is gold");
assert.strictEqual(
  finalizeOrder(cartOf([{ sku: "GOLD", priceCents: 10000 }])).totalCents,
  9720,
  "10000 - 10% + 8% tax on the net"
);
assert.strictEqual(discountRate(9999), 0.05, "99.99 is silver");
assert.strictEqual(discountRate(4999), 0, "49.99 gets no discount");

// Silver tier stays as documented; this one already passes.
assert.strictEqual(
  finalizeOrder(cartOf([{ sku: "SILVER", priceCents: 6000 }])).totalCents,
  6156,
  "6000 - 5% + 8% tax on the net"
);

// Shipping is never taxed.
const heavy = finalizeOrder(cartOf([{ sku: "H", priceCents: 1000, weightGrams: 800 }]));
assert.strictEqual(heavy.shippingCents, 750);
assert.strictEqual(heavy.totalCents, 1000 + 80 + 750, "no discount below 50.00");

console.log("pricing.test ok");
