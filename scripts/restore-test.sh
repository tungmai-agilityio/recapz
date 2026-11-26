#!/usr/bin/env bash
# Usage: restore-test.sh <backup_dir>
# Restores the newest backup stamp directory into a temp dir and checks that every SQLite file opens
# and passes integrity_check. Posts the result to ALERT_WEBHOOK (if set). Exit 1 on failure.
# Needs sqlite3, rsync, curl, jq. Run monthly (recapz-restore-test.timer).
set -u -o pipefail
src=${1:?backup_dir}
latest=$(find "$src" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)
msg=""
if [ -z "$latest" ]; then
  msg="no backup found"
else
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  rsync -a "$latest"/ "$tmp"/ || msg="copy failed"
  n=0
  while IFS= read -r f; do
    n=$((n + 1))
    [ "$(sqlite3 -readonly "$f" 'PRAGMA integrity_check' 2>&1)" = ok ] || msg="${msg:+$msg; }$(basename "$(dirname "$f")")/$(basename "$f") failed integrity check"
  done < <(find "$tmp" -name '*.sqlite')
  [ -z "$msg" ] && [ "$n" -eq 0 ] && msg="backup $(basename "$latest") holds no databases"
fi
if [ -n "$msg" ]; then text="Recapz restore test FAILED: $msg"; else text="Recapz restore test ok: $(basename "$latest"), $n databases open"; fi
echo "$text" >&2
if [ -n "${ALERT_WEBHOOK:-}" ]; then
  curl -fsS -X POST -H 'Content-Type: application/json' -d "$(jq -n --arg t "$text" '{text:$t}')" "$ALERT_WEBHOOK" >/dev/null || true
fi
[ -z "$msg" ]
