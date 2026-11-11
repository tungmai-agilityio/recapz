#!/bin/sh
# Fails if any tracked file contains a banned name. The list is kept outside the repo:
# set BANNED_NAMES to a case-insensitive extended regex (CI secret or local env var).
[ -z "$BANNED_NAMES" ] && { echo "banned-names: BANNED_NAMES not set, skipping"; exit 0; }
if git grep -n -i -E -e "$BANNED_NAMES" -- . ':!scripts/banned-names.sh'; then
  echo "banned-names: found banned names" >&2; exit 1
fi
