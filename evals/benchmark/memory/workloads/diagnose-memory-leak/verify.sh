#!/usr/bin/env bash
set -e
cd "$(dirname "$0")/seed/repo"
# Fix must use WeakValueDictionary or an explicit LRU cap.
grep -q -E 'WeakValueDictionary|lru_cache|functools.lru_cache' cache.py
