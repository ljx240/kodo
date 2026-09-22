#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
# vol must reuse area (not a manual loop)
grep -Eq "area\(" math.js
grep -Eq "return area\(|area\(w,\s*h\)|area\(w,h\)" math.js
# must not still contain the for-loop accumulation
if grep -q "for (let i = 0; i < d" math.js; then
  echo "manual loop still present"
  exit 1
fi
node test.js
