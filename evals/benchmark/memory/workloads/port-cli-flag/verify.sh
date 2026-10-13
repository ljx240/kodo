#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/seed/repo"
# config.toml must have a strict key
grep -q '^strict' config.toml
# env var still works
APP_STRICT=1 go run main.go | grep -q "strict mode on"
# help text mentions strict
grep -q -i strict README.md
