#!/usr/bin/env bash
# Usage: backup.sh <data_dir> <backup_dir> [keep_days=14]
# Consistent copy of every SQLite file (online .backup) plus recordings, into <backup_dir>/<UTC stamp>.
# Backups hold real meeting data: the directory is created 0700. Keep it off the server's own disk.
set -euo pipefail
data=${1:?data_dir}; out=${2:?backup_dir}; keep=${3:-14}
dest="$out/$(date -u +%Y%m%dT%H%M%SZ)"
umask 077
mkdir -p "$dest"
cd "$data"
find . -name '*.sqlite' | while read -r f; do
  mkdir -p "$dest/$(dirname "$f")"
  sqlite3 "$f" ".backup '$dest/$f'"
done
# everything else (recordings); skip SQLite files and their -wal/-shm sidecars, and the notetaker's
# saved Google sign-in and per-call browser copies (live session cookies never go into a backup)
find . -type f ! -name '*.sqlite' ! -name '*.sqlite-wal' ! -name '*.sqlite-shm' ! -path './notetaker-profile/*' ! -path './notetaker-tmp/*' ! -path './notetaker-debug/*' -print0 | rsync -a --from0 --files-from=- . "$dest/"
find "$out" -mindepth 1 -maxdepth 1 -type d -mtime +"$keep" -exec rm -rf {} +
echo "$dest"
