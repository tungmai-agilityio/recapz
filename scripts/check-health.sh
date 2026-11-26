#!/usr/bin/env bash
# Usage: check-health.sh <data_dir>
# Finds failed notetaker joins and jobs stuck past a limit in every tenant database (read-only) and
# posts one message to ALERT_WEBHOOK (if set) with meeting/job IDs only, never titles or errors.
# Env: STUCK_MINUTES (default 60: a job locked or queued longer than this is stuck),
# WINDOW_MINUTES (default 70: only joins that failed this recently; run hourly, so none repeats much).
# Exit 1 when something is wrong. Needs sqlite3, curl, jq.
set -u -o pipefail
data=${1:?data_dir}
stuck=${STUCK_MINUTES:-60}; win=${WINDOW_MINUTES:-70}
case "$stuck$win" in '' | *[!0-9]*) echo "STUCK_MINUTES and WINDOW_MINUTES must be numbers" >&2; exit 2 ;; esac
now=$(($(date +%s) * 1000))
msgs=()
for db in "$data"/tenants/*/recapz.sqlite; do
  [ -f "$db" ] || continue
  t=$(basename "$(dirname "$db")")
  q() { sqlite3 -readonly "$db" "$1" 2>/dev/null | paste -sd, -; }
  r=$(q "SELECT id FROM meetings WHERE status='failed' AND platform!='upload' AND created_at > $now - $win*60000")
  [ -n "$r" ] && msgs+=("tenant $t: failed joins, meetings $r")
  r=$(q "SELECT id FROM jobs WHERE (state='running' AND locked_until < $now - $stuck*60000) OR (state='queued' AND run_after < $now - $stuck*60000)")
  [ -n "$r" ] && msgs+=("tenant $t: jobs stuck over ${stuck} min, jobs $r")
done
[ "${#msgs[@]}" -eq 0 ] && exit 0
text="Recapz health check: $(printf '%s; ' "${msgs[@]}")"
echo "$text" >&2
if [ -n "${ALERT_WEBHOOK:-}" ]; then
  curl -fsS -X POST -H 'Content-Type: application/json' -d "$(jq -n --arg t "$text" '{text:$t}')" "$ALERT_WEBHOOK" >/dev/null || true
fi
exit 1
