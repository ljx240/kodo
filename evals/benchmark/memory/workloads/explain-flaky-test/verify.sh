#!/usr/bin/env bash
# Runs the seeded test 10 times; passes when 10/10 are green.
# Used by the P2 benchmark harness to score `end_task_success`.
set -e
cd "$(dirname "$0")/seed/repo"
fails=0
for i in $(seq 1 10); do
    python3 -m pytest tests/test_pool.py -q || fails=$((fails+1))
done
[ "$fails" -eq 0 ]
