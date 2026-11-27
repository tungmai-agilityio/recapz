#!/usr/bin/env bash
# Offline check for offsite-backup.sh against a local restic repository; no network. Needs restic.
cd "$(dirname "$0")/.." || exit 1
command -v restic >/dev/null || { echo "test-offsite: restic not installed, skipped"; exit 0; }
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
fail=0
export RESTIC_REPOSITORY=$d/repo RESTIC_PASSWORD=test RESTIC_CACHE_DIR=$d/cache
./scripts/offsite-backup.sh "$d/empty" 2>/dev/null && { echo "offsite: missing backup dir must fail"; fail=1; }
mkdir -p "$d/b/20260101T000000Z/tenants/t1" "$d/b/20260102T000000Z/tenants/t1"
echo old > "$d/b/20260101T000000Z/tenants/t1/recapz.sqlite"
echo new > "$d/b/20260102T000000Z/tenants/t1/recapz.sqlite"
./scripts/offsite-backup.sh "$d/b" >/dev/null || { echo "offsite: first run failed"; fail=1; }
./scripts/offsite-backup.sh "$d/b" >/dev/null || { echo "offsite: second run (existing repo) failed"; fail=1; }
./scripts/offsite-backup.sh "$d/b" >/dev/null || { echo "offsite: third run failed"; fail=1; }
# same day: restic keeps the newest and, while daily slots are free, the oldest; the middle one is pruned
n=$(restic snapshots --json | jq length)
[ "$n" = 2 ] || { echo "offsite: want 2 snapshots after prune, got $n"; fail=1; }
restic restore latest --target "$d/out" >/dev/null || { echo "offsite: restore failed"; fail=1; }
[ "$(cat "$d/out/tenants/t1/recapz.sqlite" 2>/dev/null)" = new ] || { echo "offsite: restore must hold the newest stamp"; fail=1; }
[ "$fail" = 0 ] && echo "test-offsite: ok"
exit "$fail"
