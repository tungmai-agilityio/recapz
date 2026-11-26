#!/usr/bin/env bash
# Offline checks for check-health.sh and restore-test.sh against a temp data dir; no webhook, no server.
cd "$(dirname "$0")/.." || exit 1
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT
fail=0
now=$(($(date +%s) * 1000)); hour=3600000
db=$d/data/tenants/t1/recapz.sqlite; mkdir -p "$(dirname "$db")"
sqlite3 "$db" < <(cat crates/core/migrations/tenant_001.sql)
sqlite3 "$db" "
INSERT INTO meetings (id,title,platform,status,created_at) VALUES
 ('mfail','SECRET','meet','failed',$now),('mold','x','meet','failed',$now-5*$hour),
 ('mup','x','upload','failed',$now),('mok','x','meet','ready',$now);
INSERT INTO jobs (id,kind,meeting_id,state,run_after,locked_until) VALUES
 (1,'a','m','running',0,$now-2*$hour),(2,'a','m','queued',$now-2*$hour,NULL),
 (3,'a','m','running',0,$now+$hour),(4,'a','m','queued',$now,NULL),(5,'a','m','done',0,NULL);"
out=$(./scripts/check-health.sh "$d/data" 2>&1); rc=$?
[ "$rc" = 1 ] || { echo "health: want exit 1, got $rc"; fail=1; }
[ "$out" = "Recapz health check: tenant t1: failed joins, meetings mfail; tenant t1: jobs stuck over 60 min, jobs 1,2; " ] || { echo "health output: $out"; fail=1; }
sqlite3 "$db" "DELETE FROM meetings; DELETE FROM jobs;"
./scripts/check-health.sh "$d/data" || { echo "health: clean db must exit 0"; fail=1; }

./scripts/backup.sh "$d/data" "$d/bk" >/dev/null || { echo "backup failed"; fail=1; }
./scripts/restore-test.sh "$d/bk" 2>&1 | grep -q "ok: .*, 1 databases open" || { echo "restore ok case"; fail=1; }
stamp=$(ls "$d/bk" | tail -1); printf 'garbage' >"$d/bk/$stamp/tenants/t1/recapz.sqlite"
./scripts/restore-test.sh "$d/bk" >/dev/null 2>&1 && { echo "restore: corrupt backup must fail"; fail=1; }
./scripts/restore-test.sh "$d/none" >/dev/null 2>&1 && { echo "restore: no backup must fail"; fail=1; }
exit $fail
