#!/bin/sh
set -e
cd "$(dirname "$0")/repo"
node test.js
