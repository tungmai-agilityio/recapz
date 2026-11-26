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
label status:ready fef2c0 "Unblocked and waiting to be picked up"
label status:in-progress 0e8a16 "Claimed, work in a branch or PR open"
label status:done 0e8a16 "Merged or finished"
label type:feat c5def5 "New behaviour"
label type:bug c5def5 "Something is broken"
label type:task c5def5 "Docs, maintenance, internal work"
label priority:high b60205 "Do first; security, data loss or a broken main"
label priority:medium fbca04 "Normal (default)"
label priority:low cfd3d7 "Someday"
for a in server web desktop notetaker ai tooling; do label "area:$a" bfd4f2 "Commit scope: $a"; done
label needs-review 000000 "Important: a human reviews and approves before merge"
[ "$dry" = 1 ] && echo "$n labels (dry run)" || echo "$n labels created or updated"
