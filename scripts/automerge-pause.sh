#!/usr/bin/env bash
# Pauses or resumes merging without the maintainer, using the repository variable AUTOMERGE_PAUSED.
# Usage: scripts/automerge-pause.sh status | pause | resume | detect   (needs gh and GH_TOKEN)
# detect: pauses if a merged revert PR mentions a PR that carries the "Auto-merged:" audit comment.
set -uo pipefail
case "${1:-status}" in
  status) . "$(dirname "$0")/automerge-lib.sh"; automerge_paused && echo paused || echo active ;;
  pause) gh variable set AUTOMERGE_PAUSED --body 1 && echo paused ;;
  resume) gh variable delete AUTOMERGE_PAUSED && echo active ;;
  detect)
    failed=0
    while IFS=$'\t' read -r num body; do
      [ "$num" = FAILED ] && { failed=1; continue; }
      for ref in $(grep -oE '#[0-9]+' <<<"$body" | tr -d '#' | sort -u); do
        [ "$ref" = "$num" ] && continue
        comments=$(gh pr view "$ref" --json comments --jq '.comments[].body' 2>&1) || {
          case "$comments" in *"Could not resolve to a PullRequest"*) continue ;; esac # an issue number, not a PR
          failed=1; continue
        }
        if grep -q '^Auto-merged:' <<<"$comments"; then
          gh variable set AUTOMERGE_PAUSED --body 1
          echo "paused: revert #$num touches auto-merged #$ref"
          exit 0
        fi
      done
    done < <(gh pr list --state merged --search "revert in:title" --limit 50 --json number,body \
      --jq '.[] | [.number, (.body | gsub("\n"; " "))] | @tsv' || echo FAILED)
    [ "$failed" = 0 ] || { echo "could not check every revert"; exit 1; }
    echo "no auto-merged PR was reverted"
    ;;
  *) echo "usage: scripts/automerge-pause.sh status|pause|resume|detect" >&2; exit 2 ;;
esac
