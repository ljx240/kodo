#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "port" app.js
grep -q "require('./config')" app.js
node test.js
