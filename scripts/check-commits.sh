#!/bin/sh
# Usage: check-commits.sh <range> <pr-title> <branch>
# Enforces Conventional Commits and typed branch names (see CONTRIBUTING.md):
# - branch: <type>/<kebab-description>
# - PR title (becomes the squashed commit) and every commit subject: type(scope): summary, <=72 chars
# - no tool-attribution trailers on any commit
types='feat|fix|chore|docs|refactor|test|ci|perf|build|revert'
fail=0
subject_ok() {
  printf '%s\n' "$1" | grep -Eq "^($types)(\([a-z0-9_-]+\))?!?: .*[^.]$" && [ ${#1} -le 72 ]
}
printf '%s\n' "$3" | grep -Eq "^($types)/[a-z0-9][a-z0-9._-]*(/[a-z0-9][a-z0-9._-]*)*$" \
  || { echo "bad branch name: $3 (use <type>/<kebab-description>)"; fail=1; }
subject_ok "$2" || { echo "bad PR title: $2"; fail=1; }
for c in $(git rev-list --no-merges "$1"); do
  s=$(git log -1 --format=%s "$c")
  subject_ok "$s" || { echo "bad commit subject ($c): $s"; fail=1; }
  git log -1 --format=%B "$c" | grep -Eiq '^co-authored-by:|generated with' \
    && { echo "banned trailer in $c"; fail=1; }
done
exit $fail
