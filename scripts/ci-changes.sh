#!/usr/bin/env bash
# Reads changed paths (one per line, both sides of a rename) on stdin; prints "code=false" only when every path is
# documentation that cannot affect the build: docs/** or README.md. Anything else, or an empty list, prints "code=true".
# CLAUDE.md, CONTRIBUTING.md, .claude/, .github/ and other Markdown count as code: they are tooling rules and the
# auto-merge denylist already escalates them.
code=false n=0
while IFS= read -r p; do
  [ -n "$p" ] || continue
  n=$((n + 1))
  case "$p" in docs/* | README.md) ;; *) code=true ;; esac
done
[ "$n" -gt 0 ] || code=true
echo "code=$code"
