"use strict";

const { subtotal } = require("./cart");
const { discountFor } = require("./discounts");
const { salesTax } = require("./tax");
const { shippingFor } = require("./shipping");
const { formatCents } = require("./money");

// Order of operations fixed by the finance spec: discount off the subtotal,
// tax on the discounted net, shipping added last and never taxed.
function finalizeOrder(cart, options) {
  const opts = options || {};
  const net = subtotal(cart);
  const discount = opts.ignoreDiscount ? 0 : discountFor(net);
  const afterDiscount = net - discount;
  const tax = salesTax(afterDiscount, opts.taxRate);
  const shipping = shippingFor(cart.weightGrams);
  const total = afterDiscount + tax + shipping;
  return {
    subtotalCents: net,
    discountCents: discount,
    taxCents: tax,
    shippingCents: shipping,
    totalCents: total,
    display: formatCents(total),
  };
}

module.exports = { finalizeOrder };
