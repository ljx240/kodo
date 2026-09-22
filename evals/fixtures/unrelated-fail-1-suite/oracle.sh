#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
# sum must remain the same
grep -q "return a + b" sum.js
# broken unrelated assertion fixed
node test.js
