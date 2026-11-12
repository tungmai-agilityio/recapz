#!/usr/bin/env bash
# Daily smoke test: send the notetaker to a real Meet call and a real Zoom call and check that each
# meeting reaches `ready`. Run it from cron or a timer on the server, not in CI. On any failure it
# posts a message to ALERT_WEBHOOK (if set) and exits 1.
#
# Needs: RECAPZ_URL (e.g. http://localhost:8080), RECAPZ_COOKIE (a session cookie of a member, as
# "recapz_session=..."), and MEET_TEST_URL and/or ZOOM_TEST_URL (calls kept open for the test).
# Optional: ALERT_WEBHOOK, SMOKE_TIMEOUT_SECS (default 1800). `--dry-run` prints the plan only.
# Also needs curl and jq. Calls POST /api/meetings/join (added by the upcoming-meetings API) and
# GET /api/meetings/{id}; the meeting is left in the member's account afterwards.
set -u -o pipefail

dry=0
[ "${1:-}" = "--dry-run" ] && dry=1

timeout="${SMOKE_TIMEOUT_SECS:-1800}"
case "$timeout" in '' | *[!0-9]*) echo "SMOKE_TIMEOUT_SECS must be a number" >&2; exit 2 ;; esac
failures=()

alert() {
  echo "smoke: $1" >&2
  if [ -n "${ALERT_WEBHOOK:-}" ] && [ "$dry" = 0 ]; then
    curl -fsS -X POST -H 'Content-Type: application/json' \
      -d "$(jq -n --arg t "Recapz smoke test: $1" '{text:$t}')" "$ALERT_WEBHOOK" >/dev/null || true
  fi
}

check() { # name url
  local name="$1" url="$2" body id status waited=0 errs=0
  if [ "$dry" = 1 ]; then
    echo "would send the notetaker to the $name test call and wait up to ${timeout}s for status ready"
    return 0
  fi
  body=$(jq -n --arg u "$url" --arg t "Smoke test ($name)" '{url:$u,title:$t}' \
    | curl -fsS -X POST "$RECAPZ_URL/api/meetings/join" -H "Cookie: $RECAPZ_COOKIE" \
      -H 'Content-Type: application/json' -d @-) || {
    failures+=("$name: join request failed"); return 1; }
  id=$(printf '%s' "$body" | jq -r '.id // empty' 2>/dev/null)
  [ -n "$id" ] || { failures+=("$name: no meeting id in the reply"); return 1; }
  while [ "$waited" -lt "$timeout" ]; do
    if status=$(curl -fsS "$RECAPZ_URL/api/meetings/$id" -H "Cookie: $RECAPZ_COOKIE" \
      | jq -r '.status // empty'); then errs=0; else
      status=""; errs=$((errs + 1))
      [ "$errs" -ge 5 ] && { failures+=("$name: status requests keep failing"); return 1; }
    fi
    case "$status" in
      ready) echo "smoke: $name ok"; return 0 ;;
      failed|opted_out) failures+=("$name: meeting ended as $status"); return 1 ;;
    esac
    sleep 30
    waited=$((waited + 30))
  done
  failures+=("$name: still '$status' after ${timeout}s")
  return 1
}

if [ "$dry" = 0 ] && { [ -z "${RECAPZ_URL:-}" ] || [ -z "${RECAPZ_COOKIE:-}" ]; }; then
  echo "RECAPZ_URL and RECAPZ_COOKIE are required" >&2
  exit 2
fi
if [ -z "${MEET_TEST_URL:-}" ] && [ -z "${ZOOM_TEST_URL:-}" ]; then
  echo "set MEET_TEST_URL and/or ZOOM_TEST_URL" >&2
  exit 2
fi

[ -n "${MEET_TEST_URL:-}" ] && { check meet "$MEET_TEST_URL" || true; }
[ -n "${ZOOM_TEST_URL:-}" ] && { check zoom "$ZOOM_TEST_URL" || true; }

if [ "${#failures[@]}" -gt 0 ]; then
  for f in "${failures[@]}"; do alert "$f"; done
  exit 1
fi
