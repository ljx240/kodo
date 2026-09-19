#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "pad(0" test.js
node test.js
