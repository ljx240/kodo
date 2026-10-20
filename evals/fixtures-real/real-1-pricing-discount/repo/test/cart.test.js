"use strict";

const assert = require("assert");
const { createCart, addItem, subtotal, itemCount } = require("../src/cart");
const { formatCents } = require("../src/money");

const cart = createCart();
addItem(cart, { sku: "A-1", priceCents: 1999, qty: 2, weightGrams: 120 });
addItem(cart, { sku: "B-7", priceCents: 500, qty: 1, weightGrams: 30 });

assert.strictEqual(subtotal(cart), 4498, "two 19.99 items plus one 5.00 item");
assert.strictEqual(itemCount(cart), 3, "three units in the cart");
assert.strictEqual(subtotal(addItem(createCart(), { sku: "C", priceCents: 0 })), 0);
assert.strictEqual(formatCents(4498), "44.98");
assert.strictEqual(formatCents(5), "0.05");

assert.throws(() => addItem(createCart(), { sku: "X", priceCents: 19.99 }), /integer/);
assert.throws(() => addItem(createCart(), { sku: "X", priceCents: 100, qty: 0 }), /positive/);

console.log("cart.test ok");
