#!/usr/bin/env bash
# Usage: smoke-deploy.sh <base_url>      e.g. smoke-deploy.sh https://recapz.example.com
# Quick post-deploy check, no login and no provider calls: health, the embedded web app, an
# asset with its cache header, a client route, an unauthenticated API call refused, and (for
# https) plain http redirecting. Exits 1 on the first failure.
set -u -o pipefail
base=${1:?base_url}; base=${base%/}
fail() { echo "FAIL: $1" >&2; exit 1; }
ok() { echo "ok: $1"; }
code() { curl -s -o /dev/null --max-time 10 -w '%{http_code}' "$@"; }

[ "$(code "$base/api/health")" = 200 ] || fail "/api/health is not 200"; ok "health"
home=$(curl -fsS --max-time 10 "$base/") || fail "/ did not load"
asset=$(printf '%s' "$home" | grep -o '/assets/[^"]*\.js' | head -1)
[ -n "$asset" ] || fail "/ has no script tag: web app not embedded?"; ok "web app served"
hdr=$(curl -fsSI --max-time 10 "$base$asset") || fail "$asset did not load"
printf '%s' "$hdr" | grep -qi 'cache-control:.*immutable' || fail "$asset is not cached immutable"; ok "asset"
[ "$(code "$base/meetings/1")" = 200 ] || fail "client route does not fall back to the shell"; ok "client route"
[ "$(code "$base/api/me")" = 401 ] || fail "/api/me without a session is not 401"; ok "api needs sign-in"
case "$base" in
  https://*) r=$(code "http://${base#https://}/"); case "$r" in 301|302|307|308) ok "http redirects" ;; *) fail "http answered $r, not a redirect" ;; esac ;;
esac
echo "smoke: all checks passed"
