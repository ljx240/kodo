"use strict";

const { renderInvoice } = require("./legacy/invoice");
const { renderQuote } = require("./legacy/quote");
const { titleCase } = require("./text");

function renderPacket(customer, invoice, quote) {
  return [
    titleCase(customer.name),
    renderInvoice(invoice),
    renderQuote(quote),
  ].join("\n");
}

module.exports = { renderPacket };
