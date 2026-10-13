#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/seed/repo"
# lodash must be gone
! grep -q '"lodash"' package.json
# source must still parse
node -c src/index.js
