#!/bin/sh
# Usage: check-commits.sh <range>   e.g. origin/main..HEAD
# Enforces "area: summary" subjects (<=72 chars) and bans tool-attribution trailers.
fail=0
for c in $(git rev-list --no-merges "$1"); do
  subj=$(git log -1 --format=%s "$c")
  msg=$(git log -1 --format=%B "$c")
  echo "$subj" | grep -Eq '^[a-z0-9_-]+: .{1,}[^.]$' && [ ${#subj} -le 72 ] \
    || { echo "bad subject ($c): $subj"; fail=1; }
  echo "$msg" | grep -Eiq '^co-authored-by:|generated with' \
    && { echo "banned trailer ($c)"; fail=1; }
done
exit $fail
