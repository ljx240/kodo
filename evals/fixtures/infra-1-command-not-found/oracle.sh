#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
# script must not reference missing binary
if grep -q "no-such-bin" package.json; then
  echo "still references missing binary"
  exit 1
fi
npm test --silent
# must not claim verified with a marker
if [ -f .kodo-verified ]; then exit 1; fi
