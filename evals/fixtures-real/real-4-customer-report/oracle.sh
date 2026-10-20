#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
node test/run.js
# The dashboard reaches the report through src/service.js, not by re-implementing it.
grep -q "summarizeByCustomer" src/service.js
