#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
node test/run.js
# ADR-004 forbids private copies of the helpers in the legacy renderers.
if grep -Eq "^function (formatDate|padAmount)" src/legacy/invoice.js src/legacy/quote.js; then
  echo "legacy renderers still define their own formatDate/padAmount" >&2
  exit 1
fi
