#!/usr/bin/env bash
# Creates or updates every issue label from docs/specs/delivery-pipeline.md. Safe to re-run.
# Usage: scripts/labels.sh [--dry-run]   (needs gh; pass GH_TOKEN for the right account)
set -euo pipefail
dry=0
[ "${1:-}" = "--dry-run" ] && dry=1
[ $# -eq 0 ] || [ "$dry" = 1 ] || { echo "usage: scripts/labels.sh [--dry-run]" >&2; exit 2; }
n=0
label() { # name color description
  n=$((n + 1))
  if [ "$dry" = 1 ]; then echo "$1"; else gh label create "$1" --color "$2" --description "$3" --force; fi
}
label kind:intent 5319e7 "A pitch to grill into a spec"
label kind:ticket 1d76db "One task, one PR"
label status:backlog cfd3d7 "Intent filed, not started"
label status:planning fbca04 "Being grilled into a spec"
label status:ready fef2c0 "Unblocked and waiting to be picked up"
label status:in-progress 0e8a16 "Claimed, work in a branch"
label status:review b60205 "PR open"
label status:needs-human d93f0b "Stopped; see the last comment"
for t in feat fix refactor chore docs perf test; do label "type:$t" c5def5 "Commit type: $t"; done
label priority:p0 b60205 "Security, data loss or a broken main"
label priority:p1 d93f0b "Next up"
label priority:p2 fbca04 "Normal (default)"
label priority:p3 cfd3d7 "Someday"
for a in server core web auth jobs meetings notetaker speech llm deploy tooling; do label "area:$a" bfd4f2 "Commit scope: $a"; done
label human-merge 000000 "Forces a manual merge"
[ "$dry" = 1 ] && echo "$n labels (dry run)" || echo "$n labels created or updated"
