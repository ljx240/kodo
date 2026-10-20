#!/bin/sh
# Reviewer's canonical fix, applied by the baselines to a throwaway copy so an
# unsolvable fixture is caught before it can be measured. Not shown to the model.
set -e
cd "$(dirname "$0")/repo"

node -e '
const fs = require("fs");
const p = "src/discounts.js";
const s = fs.readFileSync(p, "utf8");
fs.writeFileSync(p, s.replace(
  "if (subtotalCents > GOLD_CENTS)",
  "if (subtotalCents >= GOLD_CENTS)"
));
'
