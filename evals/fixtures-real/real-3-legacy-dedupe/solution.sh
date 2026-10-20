#!/bin/sh
# Reviewer's canonical fix for ADR-004: src/text.js owns both helpers and the
# legacy renderers require them instead of keeping private copies. Applied to a
# throwaway copy by the baselines; never copied into the model's work dir.
set -e
cd "$(dirname "$0")/repo"

node -e '
const fs = require("fs");

const text = "src/text.js";
const t = fs.readFileSync(text, "utf8");
const helpers = `function formatDate(iso) {
  const parts = String(iso).split("-");
  return \`\${parts[2]} \${MONTHS[Number(parts[1]) - 1]} \${parts[0]}\`;
}

function padAmount(value) {
  return String(value).padStart(10, " ");
}

module.exports = { MONTHS, titleCase, collapseSpaces, formatDate, padAmount };
`;
if (!t.includes("module.exports = { MONTHS, titleCase, collapseSpaces };\n")) {
  throw new Error("text.js anchor moved");
}
fs.writeFileSync(text, t.replace(
  "module.exports = { MONTHS, titleCase, collapseSpaces };\n",
  helpers
));

fs.writeFileSync("src/legacy/invoice.js", `"use strict";

const { formatDate, padAmount } = require("../text");

function renderInvoice(invoice) {
  return \`INV-\${invoice.number} | \${formatDate(invoice.issuedAt)} | total \${padAmount(invoice.total)}\`;
}

module.exports = { formatDate, padAmount, renderInvoice };
`);

fs.writeFileSync("src/legacy/quote.js", `"use strict";

const { formatDate, padAmount } = require("../text");

function renderQuote(quote) {
  return \`QTE-\${quote.number} | valid until \${formatDate(quote.validUntil)} | \${padAmount(quote.total)}\`;
}

module.exports = { formatDate, padAmount, renderQuote };
`);
'
