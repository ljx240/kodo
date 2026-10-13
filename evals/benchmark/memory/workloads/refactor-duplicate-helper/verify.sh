#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/seed/repo"
python3 -m pytest tests/test_util.py -q
