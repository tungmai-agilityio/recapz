#!/usr/bin/env bash
# Usage: sudo deploy.sh <recapz-binary> <release-name>      e.g. deploy.sh ./recapz v0.4.0
#        sudo deploy.sh --rollback                          switch back to the previous release
# Installs the binary as /opt/recapz/releases/<name>/recapz, points /usr/local/bin/recapz at it,
# restarts the service and waits for /api/health. If the new release does not come up healthy it
# switches back to the previous one. Migrations are additive, so the older binary can run on the new data.
# Build the binary elsewhere (a 4 GB host is too small for a release build); copy it here with scp.
set -euo pipefail
root=${RECAPZ_RELEASES:-/opt/recapz/releases}
link=${RECAPZ_LINK:-/usr/local/bin/recapz}
health=${RECAPZ_HEALTH_URL:-http://127.0.0.1:8080/api/health}
unit=${RECAPZ_UNIT:-recapz}
wait_secs=${RECAPZ_HEALTH_WAIT:-60}

healthy() {
  for _ in $(seq "$wait_secs"); do
    curl -fsS --max-time 2 "$health" >/dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}
switch_to() { # release dir
  ln -sfn "$1/recapz" "$link"
  systemctl restart "$unit"
}
previous() { local p; p=$(readlink "$link" 2>/dev/null) || true; [ -z "$p" ] || dirname "$p"; }

if [ "${1:-}" = "--rollback" ]; then
  cur=$(previous)
  prev=$(ls -1dt "$root"/*/ | sed 's:/$::' | grep -vx "$cur" | head -1)
  [ -n "$prev" ] || { echo "no previous release" >&2; exit 1; }
  switch_to "$prev"; healthy || { echo "rollback to $prev is not healthy" >&2; exit 1; }
  echo "rolled back to $prev"; exit 0
fi

bin=${1:?binary}; name=${2:?release name}
case "$name" in */* | .* | '') echo "bad release name" >&2; exit 2 ;; esac
[ -x "$bin" ] || { echo "$bin is not executable" >&2; exit 2; }
old=$(previous)
dir="$root/$name"
install -d -m 0755 "$dir"
install -m 0755 "$bin" "$dir/recapz"
switch_to "$dir"
if healthy; then
  echo "deployed $name"
  ls -1dt "$root"/*/ | tail -n +6 | xargs -r rm -rf   # keep the 5 newest releases
  exit 0
fi
echo "$name did not become healthy in ${wait_secs}s" >&2
if [ -n "$old" ]; then
  switch_to "$old"
  healthy && echo "switched back to $old" >&2 || echo "previous release $old is not healthy either" >&2
  rm -rf "$dir"   # a failed release must not become a rollback target
fi
exit 1
