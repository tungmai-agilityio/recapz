#!/usr/bin/env bash
# Pure checks used by scripts/automerge-check.sh; sourced, also by scripts/test-automerge.sh.
# The denylist can only force escalation; nothing here ever grants a merge on its own.

REQUIRED_CHECKS="guards rust web"

# denied_path <repo-relative path>: succeeds when a change to this path must go to the maintainer.
# Over-matching is cheap: this list only forces escalation. Meeting access in crates/core is not listed: the reviewer
# checks it and a spec-approved change is not critical (docs/specs/delivery-pipeline.md, Autonomy and escalation).
denied_path() {
  shopt -s nocasematch
  local r=1
  case "$1" in
    */migrations/*) r=0 ;;                                                                  # schema
    *auth* | *access* | *session* | *csrf* | *privacy* | *purge*) r=0 ;;
    Cargo.toml | */Cargo.toml | Cargo.lock | rust-toolchain.toml) r=0 ;;                    # dependencies and toolchain
    package.json | */package.json | pnpm-lock.yaml | */pnpm-lock.yaml | pnpm-workspace.yaml | */pnpm-workspace.yaml) r=0 ;;
    .npmrc | */.npmrc | .nvmrc | */.nvmrc) r=0 ;;
    deploy/* | scripts/* | .claude/* | .github/* | Makefile | config.example.toml) r=0 ;;
    CLAUDE.md | CONTRIBUTING.md) r=0 ;;                                                     # policy docs
  esac
  shopt -u nocasematch
  return $r
}

# decisions_empty: reads a PR body on stdin; succeeds only when "Decisions needing the maintainer" is exactly
# None (case and a trailing period ignored) once HTML comments are removed. An untouched template does not count.
decisions_empty() {
  local text
  text=$(awk '
    /^## Decisions needing the maintainer/ { on = 1; next }
    on && /^## / { exit }
    on { print }
  ' | tr -d '\r' | perl -0777 -pe 's/<!--.*?-->//gs' | tr '\n' ' ' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
  [[ "$text" =~ ^[Nn][Oo][Nn][Ee]\.?$ ]]
}

# ci_failures: reads `gh pr checks --json name,bucket` on stdin; prints every check that did not pass, and each
# required check that is missing. No output means every job passed and all required jobs ran.
ci_failures() {
  local json c
  json=$(cat)
  echo "$json" | jq -e 'type == "array"' >/dev/null 2>&1 || { echo "unreadable check results"; return; }
  echo "$json" | jq -r '.[] | select(.bucket != "pass") | .name'
  for c in $REQUIRED_CHECKS; do
    echo "$json" | jq -e --arg c "$c" 'any(.[]; .name == $c)' >/dev/null || echo "$c (missing)"
  done
}

# automerge_paused: succeeds when merging without the maintainer is paused. An API error counts as paused;
# only a clean "not found" (variable never set) or an explicit non-1 value means active.
automerge_paused() {
  local out rc
  out=$(gh variable get AUTOMERGE_PAUSED 2>&1); rc=$?
  if [ $rc -ne 0 ]; then
    # A "not found" also appears when the token cannot read variables at all, so confirm listing works.
    case "$out" in *"HTTP 404"* | *"Not Found"* | *"not found"*) gh variable list >/dev/null 2>&1 && return 1 ;; esac
    return 0
  fi
  [ "$out" = 1 ]
}
