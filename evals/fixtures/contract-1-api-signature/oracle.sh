#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
# id must be required (no default parameter)
if grep -q "id = " api.js; then
  echo "id still has a default"
  exit 1
fi
grep -q "function fetchUser(id" api.js
node test.js
