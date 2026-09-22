#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "sumAll" math.js
grep -q "sumAll" app.js
if grep -q "function total(" math.js; then
  echo "old total definition remains"
  exit 1
fi
node test.js
