#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "assert" test.js
node test.js
