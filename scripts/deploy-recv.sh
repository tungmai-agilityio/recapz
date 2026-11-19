#!/usr/bin/env bash
# Forced command for the CI deploy key (installed as /usr/local/sbin/recapz-deploy-recv, run as root through
# sudo by scripts/setup-deploy-user.sh). The key can do only what this script allows, read from
# SSH_ORIGINAL_COMMAND:
#   deploy <release-name> <sha256>   binary on stdin; checked, then handed to deploy-release.sh
#   rollback                         deploy-release.sh --rollback
#   status                           print the release now in use
# Anything else is refused. The sha256 only guards the transfer: whoever holds the key can still ship any binary.
set -euo pipefail
rel=${RECAPZ_DEPLOY_RELEASE:-/usr/local/sbin/recapz-deploy-release}
link=${RECAPZ_LINK:-/usr/local/bin/recapz}
max=$((200 * 1024 * 1024))
die() { echo "refused: $1" >&2; exit 2; }

cmd=${SSH_ORIGINAL_COMMAND:-}
[[ $cmd != *$'\n'* ]] || die "bad command"
read -r verb name sha extra <<<"$cmd" || true
case "${verb:-}" in
  status) [ -z "${name:-}" ] || die "bad command"; readlink "$link"; exit 0 ;;
  rollback) [ -z "${name:-}" ] || die "bad command"; exec "$rel" --rollback ;;
  deploy) ;;
  *) die "unknown command" ;;
esac
[[ ${name:-} =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]] || die "bad release name"
[[ ${sha:-} =~ ^[0-9a-f]{64}$ ]] || die "bad sha256"
[ -z "${extra:-}" ] || die "bad command"

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
head -c $((max + 1)) >"$tmp/recapz"
size=$(wc -c <"$tmp/recapz")
{ [ "$size" -gt 0 ] && [ "$size" -le "$max" ]; } || die "binary missing or over 200 MB"
[ "$(head -c4 "$tmp/recapz" | od -An -tx1 | tr -d ' \n')" = 7f454c46 ] || die "not a Linux executable"
[ "$(sha256sum "$tmp/recapz" | cut -d" " -f1)" = "$sha" ] || die "sha256 mismatch"
chmod 0755 "$tmp/recapz"
"$rel" "$tmp/recapz" "$name"
