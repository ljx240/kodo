#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "name" server.js
grep -q "name" client.js
node test.js
