#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
grep -q "uniqueAgain(xs) {\s*return unique" list.js || grep -Eq "uniqueAgain[^{]*\{[^}]*unique\(" list.js
node test.js
