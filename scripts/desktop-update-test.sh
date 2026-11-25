#!/usr/bin/env bash
# Local end-to-end test of the desktop updater (macOS only): builds 0.0.1 and 0.0.2, serves 0.0.2 on
# http://localhost:8765 and leaves 0.0.1 in $OUT/0.0.1 for you to open. Insecure transport is switched on for
# these two test builds only. Needs the same setup as `make desktop-dmg` (updater key, "Recapz Local").
set -euo pipefail
cd "$(dirname "$0")/.."
KEY=${UPDATER_KEY:-$HOME/.tauri/recapz-updater.key}
OUT=${OUT:-$HOME/recapz-update-test}
PORT=8765
BUNDLE=desktop/target/release/bundle/macos
[ -f "$KEY" ] || { echo "no updater key at $KEY: see docs/deploy.md"; exit 1; }
security find-certificate -c "Recapz Local" >/dev/null 2>&1 || { echo "no 'Recapz Local' certificate: run scripts/desktop-cert.sh once"; exit 1; }
mkdir -p "$OUT/0.0.1" "$OUT/serve"

build() { # version
  (cd desktop/src-tauri && CI=true RECAPZ_UPDATE_URL="http://localhost:$PORT/latest.json" APPLE_SIGNING_IDENTITY="Recapz Local" \
    TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")" cargo tauri build --bundles app \
    --config "{\"version\":\"$1\",\"plugins\":{\"updater\":{\"dangerousInsecureTransportProtocol\":true}}}")
}

[ -d "$OUT/0.0.1/Recapz.app" ] && { echo "$OUT/0.0.1/Recapz.app exists: delete it by hand and run again"; exit 1; }
build 0.0.1
cp -R "$BUNDLE/Recapz.app" "$OUT/0.0.1/Recapz.app"
build 0.0.2
cp "$BUNDLE/Recapz.app.tar.gz" "$BUNDLE/Recapz.app.tar.gz.sig" "$OUT/serve/"
ASSET_URL="http://localhost:$PORT/Recapz.app.tar.gz" scripts/desktop-latest.sh 0.0.2 "$OUT/serve/Recapz.app.tar.gz.sig" >"$OUT/serve/latest.json"

cat <<EOF

Ready. In another terminal open the old build:  open "$OUT/0.0.1/Recapz.app"
Then check:
  1. It asks "Recapz 0.0.2 is ready." Click "Restart now": the app restarts as 0.0.2 (About or Finder > Get Info).
  2. Microphone and system audio still work without a new permission prompt.
  3. xattr -l "$OUT/0.0.1/Recapz.app"  shows no com.apple.quarantine.
Serving $OUT/serve on port $PORT (Ctrl-C to stop).
EOF
exec python3 -m http.server "$PORT" --directory "$OUT/serve"
