#!/usr/bin/env bash
# Decides whether a ticket PR may be merged without the maintainer, per docs/specs/delivery-pipeline.md (Merge).
# Usage: scripts/automerge-check.sh <pr> --reviewed-sha <full 40-char sha> --implementer-risk low|high --reviewer-risk low|high|na
# `--reviewer-risk na` is accepted only when every changed file is Markdown (docs-only PRs skip the reviewer).
# Prints ELIGIBLE (exit 0) or one ESCALATE line per reason (exit 1). Never merges. Needs gh, jq, perl and GH_TOKEN.
# Every failure to read data counts as a reason to escalate. Merge only with `gh pr merge --match-head-commit <sha>`.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
. scripts/automerge-lib.sh

pr="${1:?usage: automerge-check.sh <pr> --reviewed-sha <sha> --implementer-risk low|high --reviewer-risk low|high|na}"
shift
sha="" impl="" rev=""
while [ $# -gt 0 ]; do
  case "$1" in
    --reviewed-sha) sha="${2:-}"; shift 2 ;;
    --implementer-risk) impl="${2:-}"; shift 2 ;;
    --reviewer-risk) rev="${2:-}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

reasons=()
no() { reasons+=("$1"); }

if automerge_paused; then no "auto-merge is paused or its state cannot be read (AUTOMERGE_PAUSED)"; fi
scripts/automerge-pause.sh detect >/dev/null 2>&1 || no "cannot check for reverted auto-merges"
automerge_paused && no "auto-merge was paused by a revert"

info=$(gh pr view "$pr" --json state,isDraft,baseRefName,headRefOid,labels,body) || { echo "ESCALATE: cannot read PR $pr"; exit 1; }
[ "$(jq -r .state <<<"$info")" = OPEN ] || no "PR is not open"
[ "$(jq -r .isDraft <<<"$info")" = false ] || no "PR is a draft"
[ "$(jq -r .baseRefName <<<"$info")" = main ] || no "PR does not target main"
labels=$(jq -r '.labels[].name' <<<"$info") || no "cannot read labels"
grep -qx needs-review <<<"$labels" && no "labelled needs-review"
head=$(jq -r .headRefOid <<<"$info")
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || no "reviewed sha must be the full 40-character commit id"
[ "$sha" = "$head" ] || no "reviewer approval is not for the current HEAD ($head)"
[ "$impl" = low ] || no "implementer did not grade the risk low"

jq -r .body <<<"$info" | decisions_empty || no "Decisions needing the maintainer is not exactly None"

# Both names of a renamed file are checked; an empty or failed listing, or one at the API cap, escalates.
if files=$(gh api --paginate "repos/{owner}/{repo}/pulls/$pr/files" \
  --jq '.[] | "F " + .filename, (if .previous_filename then "P " + .previous_filename else empty end)') &&
  [ -n "$files" ]; then
  [ "$(grep -c '^F ' <<<"$files")" -lt 3000 ] || no "changed-file list may be truncated (3000 files)"
  nondoc=0
  while IFS= read -r line; do
    denied_path "${line#? }" && no "denylisted path: ${line#? }"
    [[ "${line#? }" == *.md ]] || nondoc=1
  done <<<"$files"
  if [ "$rev" = na ]; then
    [ "$nondoc" = 0 ] || no "reviewer risk na is only for docs-only PRs"
  else
    [ "$rev" = low ] || no "reviewer did not grade the risk low"
  fi
else
  no "cannot list the changed files"
  [ "$rev" = low ] || no "reviewer did not grade the risk low"
fi

checks=$(gh pr checks "$pr" --json name,bucket) || checks=""
while IFS= read -r c; do
  [ -n "$c" ] && no "CI check did not pass on HEAD: $c"
done < <(ci_failures <<<"${checks:-null}")

# The head must not have moved while the checks above were read.
head2=$(gh pr view "$pr" --json headRefOid --jq .headRefOid) || head2=""
[ "$head2" = "$head" ] || no "HEAD changed while checking ($head -> ${head2:-unknown})"

if [ ${#reasons[@]} -eq 0 ]; then
  echo ELIGIBLE
  exit 0
fi
printf 'ESCALATE: %s\n' "${reasons[@]}"
exit 1
