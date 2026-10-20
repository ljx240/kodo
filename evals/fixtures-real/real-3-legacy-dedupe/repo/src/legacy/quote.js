"use strict";

const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

function formatDate(iso) {
  const parts = iso.split("-");
  return `${parts[2]} ${MONTHS[Number(parts[1])]} ${parts[0]}`;
}

function padAmount(value) {
  return String(value).padStart(10, " ");
}

function renderQuote(quote) {
  return `QTE-${quote.number} | valid until ${formatDate(quote.validUntil)} | ${padAmount(quote.total)}`;
}

module.exports = { formatDate, padAmount, renderQuote };
