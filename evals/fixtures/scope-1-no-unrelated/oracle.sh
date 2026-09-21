#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "KEEP_ME_UNRELATED" notes.txt
node test.js
