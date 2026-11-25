#!/usr/bin/env bash
# Checks scripts/desktop-latest.sh output and that the pre-commit hook refuses a private key.
# Scratch files live under the system temp dir (the OS clears it).
cd "$(dirname "$0")/.." || exit 1
fail=0
hook=$PWD/scripts/git-hooks/pre-commit
dir=$(mktemp -d)
printf 'c2lnLWJhc2U2NA==\n' >"$dir/x.sig"
got=$(PUB_DATE=2026-01-02T03:04:05Z scripts/desktop-latest.sh 0.1.3 "$dir/x.sig" arm64)
want='{"version":"0.1.3","notes":"Recapz 0.1.3","pub_date":"2026-01-02T03:04:05Z","platforms":{"darwin-aarch64":{"signature":"c2lnLWJhc2U2NA==","url":"https://github.com/tungmai-agilityio/recapz/releases/download/v0.1.3/Recapz.app.tar.gz"}}}'
[ "$got" = "$want" ] || { echo "latest.json differs: $got"; fail=1; }
: >"$dir/empty.sig"
scripts/desktop-latest.sh 0.1.3 "$dir/empty.sig" arm64 2>/dev/null && { echo "empty signature accepted"; fail=1; }

commit_in() { # file content: stage it in a fresh scratch repo and run the hook there
  local r
  r=$(mktemp -d) && (cd "$r" && git init -q && printf '%s\n' "$2" >"$1" && git add -f "$1" &&
    GIT_AUTHOR_NAME=t GIT_COMMITTER_NAME=t GIT_AUTHOR_EMAIL=a@asnet.com.vn GIT_COMMITTER_EMAIL=a@asnet.com.vn "$hook" 2>/dev/null)
}
commit_in recapz.key "anything" && { echo "a .key file was accepted"; fail=1; }
commit_in notes.txt "untrusted comment: minisign encrypted secret key" && { echo "a minisign secret key was accepted"; fail=1; }
commit_in notes.txt "hello" || { echo "a normal file was refused"; fail=1; }
[ "$fail" = 0 ] && echo "desktop-latest: ok"
exit "$fail"
