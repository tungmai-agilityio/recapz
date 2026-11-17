#!/usr/bin/env bash
# Checks .claude/hooks/guard-push.sh and scripts/git-hooks/pre-push against known commands and refs.
cd "$(dirname "$0")/.." || exit 1
fail=0 n=0
guard() { # expected-exit command
  n=$((n + 1))
  jq -n --arg c "$2" '{tool_input: {command: $c}}' | .claude/hooks/guard-push.sh 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "guard-push: want $1, got $got: $2"; fail=1; }
}
prepush() { # expected-exit remote-ref
  n=$((n + 1))
  echo "refs/heads/x abc $2 def" | scripts/git-hooks/pre-push 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "pre-push: want $1, got $got: $2"; fail=1; }
}
guard 0 'git push -u origin chore/guard-force-push'
guard 0 'git push --force-with-lease origin feat/x'
guard 0 'git push -u --force-with-lease origin feat/x'
guard 0 'git push --force-with-lease --force-if-includes origin feat/x'
guard 0 'git push origin feat/maintenance'
guard 0 'git config --get core.hooksPath'
guard 0 'git config core.hooksPath scripts/git-hooks'
guard 0 'grep -n hooksPath Makefile'
guard 0 'cargo test -p core'
guard 2 'git push --force origin feat/x'
guard 2 'git push -f origin feat/x'
guard 2 'git push -uf origin feat/x'
guard 2 'git push origin feat/x --force'
guard 2 'git push --mirror origin'
guard 2 'git push origin +feat/x'
guard 2 'git push origin main'
guard 2 'git push origin HEAD:main'
guard 2 'git push origin feat/x:refs/heads/main'
guard 2 'git push --force-with-lease origin main'
guard 2 'git push origin --delete main'
guard 2 'git -C web push -f origin feat/x'
guard 2 'cd web && git push --force origin feat/x'
guard 2 'git push --no-verify origin feat/x'
guard 2 'git -c core.hooksPath=/dev/null push origin feat/x'
guard 2 'git config --unset core.hooksPath'
prepush 0 refs/heads/feat/x
prepush 0 refs/heads/main-notes
prepush 1 refs/heads/main
[ "$n" = 28 ] || { echo "ran $n cases, want 28"; fail=1; }
[ "$fail" = 0 ] && echo "push guards: $n cases pass"
exit "$fail"
