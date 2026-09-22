#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
node test.js
if [ -f .kodo-verified ]; then
  # marker only allowed if tests already passed above
  :
fi
