#!/bin/sh
# Reviewer's canonical fix (two independent bugs). Applied to a throwaway copy
# by the baselines; never copied into the model's work dir.
set -e
cd "$(dirname "$0")/repo"

node -e '
const fs = require("fs");

const stock = "src/stock.js";
const s = fs.readFileSync(stock, "utf8");
fs.writeFileSync(stock, s.replace(
  `if (record.status === "confirmed" || record.status === "pending") held += 1;`,
  `if (record.status === "confirmed") held += 1;`
));

const reservations = "src/reservations.js";
const r = fs.readFileSync(reservations, "utf8");
if (!r.includes("if (left > 1)")) throw new Error("reservations.js anchor moved");
fs.writeFileSync(reservations, r.replace("if (left > 1)", "if (left >= 1)"));
'
