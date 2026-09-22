#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "8080" app.js
node test.js
