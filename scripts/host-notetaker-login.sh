#!/usr/bin/env bash
# Usage: scripts/host-notetaker-login.sh <droplet-ip>     (on your Mac; RECAPZ_SSH_KEY, default ~/.ssh/recapz_do)
# Signs the notetaker's Google account in on the host: opens the sign-in window on the host's virtual display,
# shows it here through an SSH tunnel and VNC (listening on localhost only, one-time password), and cleans up
# when you close the window. Runs Chromium as `recapz` only: a root-owned profile file breaks every call.
set -euo pipefail
ip=${1:?usage: host-notetaker-login.sh <droplet-ip>}
key=${RECAPZ_SSH_KEY:-$HOME/.ssh/recapz_do}
port=${VNC_PORT:-5901}
host=(ssh -i "$key" -o BatchMode=yes "root@$ip")
pw=$(openssl rand -hex 4)
pwfile=/var/lib/recapz/.vnc-once

# the password goes through stdin; x11vnc deletes the file once it has read it (rm: prefix)
printf '%s\n' "$pw" | "${host[@]}" "umask 077 && cat > $pwfile && chown recapz:recapz $pwfile"
"${host[@]}" -o ExitOnForwardFailure=yes -L "$port:localhost:5900" \
  "sudo -u recapz x11vnc -display :99 -auth /var/lib/recapz/xauthority -localhost -once -timeout 900 -rfbport 5900 -passwdfile rm:$pwfile -quiet >/dev/null 2>&1" &
tunnel=$!
cleanup() {
  kill "$tunnel" 2>/dev/null || true
  "${host[@]}" "pkill -u recapz x11vnc; rm -f $pwfile; find /var/lib/recapz/notetaker-profile ! -user recapz -print -quit" |
    grep -q . && echo "warning: files in notetaker-profile not owned by recapz: chown -R recapz:recapz /var/lib/recapz/notetaker-profile" >&2
  return 0
}
trap cleanup EXIT
sleep 3
echo "VNC password (one time): $pw"
command -v open >/dev/null && open "vnc://localhost:$port" || echo "open vnc://localhost:$port in a VNC viewer"
echo "Sign in to the notetaker's Google account in the window, then close it."
"${host[@]}" 'sudo -u recapz env DISPLAY=:99 XAUTHORITY=/var/lib/recapz/xauthority RECAPZ_CONFIG=/etc/recapz/config.toml HOME=/var/lib/recapz sh -c "cd /var/lib/recapz && recapz notetaker-login" 2>/dev/null | tail -1'
