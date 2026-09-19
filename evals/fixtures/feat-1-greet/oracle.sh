#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
test -f greet.js
grep -q "greet" greet.js
node test.js
