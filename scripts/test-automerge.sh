#!/usr/bin/env bash
# Offline checks for scripts/automerge-lib.sh: denylist paths, the decisions section and the CI result parser.
cd "$(dirname "$0")/.." || exit 1
. scripts/automerge-lib.sh
fail=0 n=0
ok() { # description command...
  n=$((n + 1)); local d="$1"; shift
  "$@" >/dev/null 2>&1 || { echo "want success: $d"; fail=1; }
}
no() { # description command...
  n=$((n + 1)); local d="$1"; shift
  "$@" >/dev/null 2>&1 && { echo "want failure: $d"; fail=1; }
}
no_denied() { ! denied_path "$1"; }
for p in web/src/pages/Meetings.tsx web/src/ui/Button.tsx crates/notetaker/src/meet.rs crates/speech/src/transcribe.rs \
  docs/ui-parity.md docs/deploy.md README.md web/src/index.css crates/core/src/db.rs crates/llm/prompts/report.md \
  crates/server/src/routes.rs crates/server/src/routes/ask.rs web/src/api.ts crates/server/src/runner.rs \
  crates/server/tests/api.rs crates/llm/src/ask.rs crates/eval/src/lib.rs docs/specs/new.md docs/decisions/x.md; do ok "allowed path $p" no_denied "$p"; done
for p in crates/core/migrations/tenant_006.sql crates/server/src/auth.rs \
  web/src/pages/Access.tsx Cargo.toml crates/speech/Cargo.toml Cargo.lock web/package.json web/pnpm-lock.yaml \
  rust-toolchain.toml deploy/recapz.service scripts/labels.sh .claude/hooks/x.sh .claude/skills/y/SKILL.md \
  .github/workflows/ci.yml Makefile CLAUDE.md CONTRIBUTING.md Crates/Core/migrations/x.sql \
  pnpm-workspace.yaml web/.npmrc .nvmrc; do
  ok "denied path $p" denied_path "$p"; done
body() { printf '## Summary\nx\n\n## Decisions needing the maintainer\n%s\n\n## Checklist\n- [ ] y\n' "$1"; }
ok "None" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\nNone\n' | decisions_empty"
ok "none." bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\nnone.\n## Checklist\n' | decisions_empty"
ok "comment then None" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\n<!-- a\nb -->\nNone\n' | decisions_empty"
no "untouched template" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\n<!-- a\nb -->\n\n## Checklist\n' | decisions_empty"
no "a real decision" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\nUsed a 3 second poll.\n' | decisions_empty"
no "no section" bash -c ". scripts/automerge-lib.sh; printf '## Summary\nx\n' | decisions_empty"
no "None plus non-latin text" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\nNone \xe2\x80\x94 \xe6\xb1\xba\xe5\xae\x9a\n' | decisions_empty"
no "text between two comments" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\nNone <!-- a --> but decide X <!-- b -->\n' | decisions_empty"
ok "None with a comment on each side" bash -c ". scripts/automerge-lib.sh; printf '## Decisions needing the maintainer\n<!-- a -->None<!-- b -->\n' | decisions_empty"
all_pass='[{"name":"commits","bucket":"pass"},{"name":"size","bucket":"pass"},{"name":"guards","bucket":"pass"},{"name":"rust","bucket":"pass"},{"name":"web","bucket":"pass"}]'
ok "all pass" test -z "$(echo "$all_pass" | ci_failures)"
no "a skipped job" test -z "$(echo "${all_pass/\"web\",\"bucket\":\"pass\"/\"web\",\"bucket\":\"skipping\"}" | ci_failures)"
no "a failed job" test -z "$(echo "${all_pass/\"rust\",\"bucket\":\"pass\"/\"rust\",\"bucket\":\"fail\"}" | ci_failures)"
no "an extra failing job" test -z "$(echo "${all_pass%]},{\"name\":\"deploy\",\"bucket\":\"fail\"}]" | ci_failures)"
no "garbage results" test -z "$(echo 'null' | ci_failures)"
no "a missing job" test -z "$(echo '[{"name":"commits","bucket":"pass"}]' | ci_failures)"
no "a pending job" test -z "$(echo "${all_pass/\"size\",\"bucket\":\"pass\"/\"size\",\"bucket\":\"pending\"}" | ci_failures)"

# --- automerge-check.sh itself, with a stub gh that serves canned data from $GH_FIX
stub=$(mktemp -d); trap 'rm -rf "$stub"' EXIT
cat > "$stub/gh" <<'STUB'
#!/usr/bin/env bash
F=$GH_FIX
case "$1 $2" in
  "variable get")
    [ -f "$F/var.err" ] && { cat "$F/var.err" >&2; exit 1; }
    [ -f "$F/var" ] && { cat "$F/var"; exit 0; }
    echo "HTTP 404: Not Found" >&2; exit 1 ;;
  "pr view")
    if [[ "$*" == *"--jq .headRefOid"* ]]; then
      if [ -f "$F/head2" ]; then cat "$F/head2"; else jq -r .headRefOid "$F/view.json"; fi
    else cat "$F/view.json"; fi ;;
  "api --paginate") [ -f "$F/files.fail" ] && exit 1; cat "$F/files.txt" ;;
  "pr checks") [ -f "$F/checks.fail" ] && exit 1; cat "$F/checks.json" ;;
  "variable list") [ -f "$F/varlist.fail" ] && exit 1; exit 0 ;;
  "pr list") exit 0 ;;
  *) echo "stub gh: unexpected: $*" >&2; exit 99 ;;
esac
STUB
chmod +x "$stub/gh"
sha=$(printf 'a%.0s' $(seq 40)); sha2=$(printf 'b%.0s' $(seq 40))
scenario() { # description expected-exit [setup-snippet using $F] [extra args to automerge-check.sh]
  n=$((n + 1))
  local F; F=$(mktemp -d)
  printf '{"state":"OPEN","isDraft":false,"baseRefName":"main","headRefOid":"%s","labels":[],"body":"## Summary\\nx\\n\\n## Decisions needing the maintainer\\nNone\\n"}' "$sha" > "$F/view.json"
  printf 'F web/src/pages/Meetings.tsx\n' > "$F/files.txt"
  echo "$all_pass" > "$F/checks.json"
  eval "${3:-:}"
  local args=("${@:4}"); [ ${#args[@]} -gt 0 ] || args=(--reviewed-sha "$sha" --implementer-risk low --reviewer-risk low)
  GH_FIX="$F" PATH="$stub:$PATH" scripts/automerge-check.sh 42 "${args[@]}" >/dev/null 2>&1
  local got=$?
  [ "$got" = "$2" ] || { echo "automerge-check ($1): want exit $2, got $got"; fail=1; }
  rm -rf "$F"
}
scenario "everything good" 0
scenario "gh api for files fails" 1 'touch $F/files.fail'
scenario "empty file list" 1 ': > $F/files.txt'
scenario "rename out of migrations" 1 'printf "F crates/speech/src/x.rs\nP crates/core/migrations/x.sql\n" > $F/files.txt'
scenario "denylisted migration" 1 'echo "F crates/core/migrations/tenant_006.sql" > $F/files.txt'
scenario "core file is not denylisted" 0 'echo "F crates/core/src/db.rs" > $F/files.txt'
scenario "an extra failing check" 1 'echo "${all_pass%]},{\"name\":\"deploy\",\"bucket\":\"fail\"}]" > $F/checks.json'
scenario "web skipped" 1 'echo "${all_pass/\"web\",\"bucket\":\"pass\"/\"web\",\"bucket\":\"skipping\"}" > $F/checks.json'
scenario "checks call fails" 1 'touch $F/checks.fail'
scenario "head moves between reads" 1 'echo "$sha2" > $F/head2'
scenario "paused" 1 'echo 1 > $F/var'
scenario "variable not found but listing fails" 1 'touch $F/varlist.fail'
scenario "variable API error" 1 'echo "HTTP 500: server error" > $F/var.err'
scenario "human-merge label" 1 'jq ".labels=[{\"name\":\"human-merge\"}]" $F/view.json > $F/v && mv $F/v $F/view.json'
scenario "draft" 1 'jq ".isDraft=true" $F/view.json > $F/v && mv $F/v $F/view.json'
scenario "a real decision in the body" 1 'jq ".body=\"## Decisions needing the maintainer\nPolls every 3s.\n\"" $F/view.json > $F/v && mv $F/v $F/view.json'
scenario "reviewed sha is not HEAD" 1 ':' --reviewed-sha "$sha2" --implementer-risk low --reviewer-risk low
scenario "short sha" 1 ':' --reviewed-sha "${sha:0:7}" --implementer-risk low --reviewer-risk low
scenario "implementer risk high" 1 ':' --reviewed-sha "$sha" --implementer-risk high --reviewer-risk low
scenario "reviewer risk high" 1 ':' --reviewed-sha "$sha" --implementer-risk low --reviewer-risk high
scenario "docs-only PR, no reviewer" 0 'echo "F docs/ui-parity.md" > $F/files.txt' --reviewed-sha "$sha" --implementer-risk low --reviewer-risk na
scenario "reviewer na with code" 1 ':' --reviewed-sha "$sha" --implementer-risk low --reviewer-risk na
scenario "reviewer na with CLAUDE.md" 1 'echo "F CLAUDE.md" > $F/files.txt' --reviewed-sha "$sha" --implementer-risk low --reviewer-risk na
[ "$n" = 79 ] || { echo "ran $n cases, want 79"; fail=1; }
[ "$fail" = 0 ] && echo "automerge: $n cases pass"
exit "$fail"
