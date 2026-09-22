#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "USER_DIRTY_MARKER" user_notes.md
node test.js
