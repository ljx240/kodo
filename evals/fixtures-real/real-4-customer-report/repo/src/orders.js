"use strict";

// Read-only seed data for the demo tenant. Reports must not mutate it.
const ORDERS = [
  { id: "o-1", customerId: "c-2", totalCents: 4500, status: "paid", placedAt: "2026-09-01" },
  { id: "o-2", customerId: "c-1", totalCents: 1200, status: "paid", placedAt: "2026-09-02" },
  { id: "o-3", customerId: "c-2", totalCents: 500, status: "cancelled", placedAt: "2026-09-03" },
  { id: "o-4", customerId: "c-3", totalCents: 2400, status: "refunded", placedAt: "2026-09-04" },
  { id: "o-5", customerId: "c-1", totalCents: 900, status: "paid", placedAt: "2026-09-05" },
  { id: "o-6", customerId: "c-3", totalCents: 5400, status: "paid", placedAt: "2026-09-06" },
];

function listOrders() {
  return ORDERS.map((order) => ({ ...order }));
}

function ordersFor(customerId) {
  return ORDERS.filter((order) => order.customerId === customerId).map((order) => ({ ...order }));
}

module.exports = { listOrders, ordersFor };
