"use strict";

function createCart() {
  return { items: [], weightGrams: 0 };
}

function addItem(cart, item) {
  if (!item || !Number.isInteger(item.priceCents) || item.priceCents < 0) {
    throw new Error("item.priceCents must be a non-negative integer");
  }
  const qty = item.qty === undefined ? 1 : item.qty;
  if (!Number.isInteger(qty) || qty <= 0) {
    throw new Error("item.qty must be a positive integer");
  }
  const existing = cart.items.find(
    (row) => row.sku === item.sku && row.priceCents === item.priceCents
  );
  if (existing) {
    existing.qty += qty;
  } else {
    cart.items.push({ sku: item.sku, priceCents: item.priceCents, qty });
  }
  cart.weightGrams += item.weightGrams === undefined ? 0 : item.weightGrams * qty;
  return cart;
}

function subtotal(cart) {
  return cart.items.reduce((total, row) => total + row.priceCents * row.qty, 0);
}

function itemCount(cart) {
  return cart.items.reduce((total, row) => total + row.qty, 0);
}

module.exports = { createCart, addItem, subtotal, itemCount };
