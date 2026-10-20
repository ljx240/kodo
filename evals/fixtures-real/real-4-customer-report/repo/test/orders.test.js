"use strict";

// Green today: the seed store itself is not part of the ticket.
const assert = require("assert");
const { listOrders, ordersFor } = require("../src/orders");

assert.strictEqual(listOrders().length, 6);
assert.strictEqual(ordersFor("c-1").length, 2);
assert.deepStrictEqual(ordersFor("nobody"), []);

const rows = listOrders();
rows[0].totalCents = 1;
assert.strictEqual(listOrders()[0].totalCents, 4500, "callers cannot mutate the store");

console.log("orders.test ok");
