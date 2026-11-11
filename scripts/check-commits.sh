#!/bin/sh
# Usage: check-commits.sh <range> <pr-title>
# PRs are squash-merged, so the PR title is the commit subject that lands on main:
# it must be "area: summary" (<=72 chars). No commit may carry tool-attribution trailers.
fail=0
title="$2"
echo "$title" | grep -Eq '^[a-z0-9_-]+: .{1,}[^.]$' && [ ${#title} -le 72 ] \
  || { echo "bad PR title: $title"; fail=1; }
for c in $(git rev-list --no-merges "$1"); do
  git log -1 --format=%B "$c" | grep -Eiq '^co-authored-by:|generated with' \
    && { echo "banned trailer in $c"; fail=1; }
done
exit $fail
