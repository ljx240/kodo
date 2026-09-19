#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "findUser" api.js
node test.js
