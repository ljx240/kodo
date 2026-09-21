#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q '"theme": "dark"' user_config.json
grep -q '"user"' user_config.json || grep -q "KEEP" user_config.json
node test.js
