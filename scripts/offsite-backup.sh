#!/usr/bin/env bash
# Usage: offsite-backup.sh <backup_dir>
# Pushes the newest local backup stamp (written by backup.sh) to an encrypted restic repository, creating it on
# the first run, then keeps 7 daily and 4 weekly snapshots. Settings come from the environment
# (/etc/recapz/offsite.env): RESTIC_REPOSITORY (e.g. s3:https://<endpoint>/<bucket>), RESTIC_PASSWORD,
# AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY. Without RESTIC_PASSWORD the backup cannot be read: keep it in a password
# manager, apart from the backups. Run daily after the local backup (recapz-offsite.timer).
set -euo pipefail
src=${1:?backup_dir}
latest=$(find "$src" -mindepth 1 -maxdepth 1 -type d | sort | tail -1)
[ -n "$latest" ] || { echo "offsite: no backup found in $src" >&2; exit 1; }
restic cat config >/dev/null 2>&1 || restic init
# each stamp is a different path, so snapshots are grouped by host and tag, not by path
cd "$latest" && restic backup --host recapz --tag recapz .
restic forget --host recapz --tag recapz --group-by host,tags --keep-daily 7 --keep-weekly 4 --prune
