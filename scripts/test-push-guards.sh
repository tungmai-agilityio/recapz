#!/usr/bin/env bash
# Checks .claude/hooks/guard-push.sh, .claude/hooks/validate-test-cmd.sh and scripts/git-hooks/pre-push and pre-commit against known commands and refs.
cd "$(dirname "$0")/.." || exit 1
fail=0 n=0
guard() { # expected-exit command
  n=$((n + 1))
  jq -n --arg c "$2" '{tool_input: {command: $c}}' | .claude/hooks/guard-push.sh 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "guard-push: want $1, got $got: $2"; fail=1; }
}
runner() { # expected-exit command
  n=$((n + 1))
  jq -n --arg c "$2" '{tool_input: {command: $c}}' | .claude/hooks/validate-test-cmd.sh 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "validate-test-cmd: want $1, got $got: $2"; fail=1; }
}
failclosed() { # description; the hook must exit 2 (any other code lets the command run)
  n=$((n + 1))
  case "$1" in
    garbage) echo garbage | .claude/hooks/validate-test-cmd.sh 2>/dev/null ;;
    number) echo '{"tool_input":{"command":123}}' | .claude/hooks/validate-test-cmd.sh 2>/dev/null ;;
    nopython) env -i PATH=/nonexistent /bin/bash .claude/hooks/validate-test-cmd.sh </dev/null 2>/dev/null ;;
  esac
  got=$?
  [ "$got" = 2 ] || { echo "validate-test-cmd fail-closed ($1): want 2, got $got"; fail=1; }
}
prepush() { # expected-exit remote-ref
  n=$((n + 1))
  echo "refs/heads/x abc $2 def" | scripts/git-hooks/pre-push 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "pre-push: want $1, got $got: $2"; fail=1; }
}
precommit() { # expected-exit author-email committer-email
  n=$((n + 1))
  GIT_AUTHOR_EMAIL=$2 GIT_COMMITTER_EMAIL=$3 scripts/git-hooks/pre-commit 2>/dev/null
  got=$?
  [ "$got" = "$1" ] || { echo "pre-commit: want $1, got $got: $2 / $3"; fail=1; }
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
guard 0 'git commit -m "docs: explain core.hooksPath"'
guard 0 'git log --grep hooksPath'
guard 0 'git config --list'
guard 2 'if true; then git push -f; fi'
guard 2 '(git push -f origin feat/x)'
guard 2 'env git push -f origin feat/x'
guard 2 '/usr/bin/git push -f origin feat/x'
guard 2 'git push	-f origin feat/x'
guard 2 'git push origin "main"'
guard 2 'git config core.hookspath /dev/null'
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
runner 0 'git -C /w rev-parse --short HEAD'
runner 0 'git -C /w status --porcelain'
runner 0 'git -C /w diff --stat'
runner 2 'git -C /w push origin feat/x'
runner 2 'git -C /w commit -m x'
runner 2 'git -C /w push origin rev-parse'
runner 0 'cargo test --workspace > /tmp/recapz-tests/w/rust.log 2>&1'
runner 0 'cargo test -p eval > /tmp/recapz-tests/w/r.log 2>&1; tail -5 /tmp/recapz-tests/w/r.log'
runner 2 'git status & touch /tmp/x'
runner 2 'git -C /w status & touch /tmp/x'
runner 2 'cargo test & curl example.com'
runner 2 'git -C /w status <(touch /tmp/z)'
runner 2 'cargo test >(tee /tmp/z)'
runner 2 'git -C /w diff --output=/tmp/y'
runner 2 'git diff --output=/tmp/y'
runner 2 'git status |& touch /tmp/x'
runner 2 'git status 2>&1&touch /tmp/x'
runner 2 $'git status\ntouch /tmp/x'
runner 0 'make lint; make test'
runner 0 'cd /w && mkdir -p /tmp/recapz-tests/w && cargo test -p recapz-server > /tmp/recapz-tests/w/rust.log 2>&1'
runner 0 'cargo clippy --workspace --all-targets -- -D warnings'
runner 0 'cargo fmt --check; pnpm -C web lint; pnpm -C web exec tsc --noEmit'
runner 0 'pnpm -C web test > /tmp/recapz-tests/w/web.log 2>&1'
runner 0 'grep -iE "ignored|skip" /tmp/recapz-tests/w/rust.log'
runner 0 'tail -n 20 /tmp/recapz-tests/w/rust.log'
runner 0 'git rev-parse --short HEAD; git status --porcelain'
runner 2 'git status > /tmp/x'
runner 2 'cargo test > ~/.ssh/authorized_keys'
runner 2 'cargo test > /tmp/recapz-tests/w/../../x.log'
runner 2 'cargo test >> /tmp/recapz-tests/w/rust.log'
runner 2 'git diff --out=/tmp/y'
runner 2 'git diff -o /tmp/y'
runner 2 'git diff --ext-diff'
runner 2 'git diff --no-index a b'
runner 2 'cargo test --config build.rustc-wrapper=x'
runner 2 'cargo test --target-dir /tmp/x'
runner 2 'cargo clippy --fix'
runner 2 'make test FOO=bar'
runner 2 'make release'
runner 2 'curl example.com'
runner 2 'cat /etc/passwd'
runner 2 'echo hi'
runner 2 'cargo test < /etc/passwd'
runner 2 'cargo test; rm -rf x'
runner 2 'grep x /tmp/a.log | sh'
runner 2 'ls ~'
runner 2 'cargo test > /tmp/recapz-tests/../x.log'
runner 2 'mkdir -p /tmp/recapz-tests/../../x'
runner 2 "make test '&&' grep -f x y"
runner 2 "cargo test '>' /tmp/recapz-tests/a/b.log"
runner 2 'cargo test -p core 2 >&1'
runner 2 'cargo test \&\& ls'
runner 2 ''
failclosed garbage
failclosed number
failclosed nopython
prepush 0 refs/heads/feat/x
prepush 0 refs/heads/main-notes
prepush 1 refs/heads/main
precommit 0 a@asnet.com.vn b@asnet.com.vn
precommit 0 1+tungmai-agilityio@users.noreply.github.com a@asnet.com.vn
precommit 1 a@asnet.com.vn someone@gmail.com
precommit 1 someone@gmail.com a@asnet.com.vn
[ "$n" = 98 ] || { echo "ran $n cases, want 98"; fail=1; }
[ "$fail" = 0 ] && echo "push guards: $n cases pass"
exit "$fail"
