"use strict";

const MONTHS = ["January","February","March","April","May","June","July","August","September","October","November","December"];

function formatDate(iso) {
  const parts = iso.split("-");
  // Legacy: US ordering, kept because the old print shop parses it.
  return `${MONTHS[Number(parts[1]) - 1]} ${parts[2]}, ${parts[0]}`;
}

function padAmount(value) {
  return String(value).padStart(10, "0");
}

function renderInvoice(invoice) {
  return `INV-${invoice.number} | ${formatDate(invoice.issuedAt)} | total ${padAmount(invoice.total)}`;
}

module.exports = { formatDate, padAmount, renderInvoice };
