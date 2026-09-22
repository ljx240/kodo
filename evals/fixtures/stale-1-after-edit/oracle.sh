#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "2.0.0" package_info.js
node test.js
