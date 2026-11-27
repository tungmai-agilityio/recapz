#!/usr/bin/env bash
# Usage: sudo scripts/provision-host.sh     (run from a checkout of this repo on a fresh Debian 12 or 13 host)
# Prepares the host: packages, service user, directories, systemd units, firewall, SSH key-only.
# Safe to run twice. It installs no binary, no config and no secrets: after it, follow docs/deploy.md
# ("Meeting notetaker", the env file, the Caddyfile, then scripts/deploy-release.sh).
# Debian is assumed on purpose: Ubuntu's `chromium` package is a snap, which does not work under this
# service's systemd hardening.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
. /etc/os-release
[ "${ID:-}" = debian ] || { echo "this script supports Debian only (found ${ID:-unknown})" >&2; exit 1; }
repo=$(cd "$(dirname "$0")/.." && pwd)
export DEBIAN_FRONTEND=noninteractive

apt-get update -y
apt-get install -y chromium xvfb x11vnc sqlite3 rsync restic curl jq ca-certificates gnupg ufw debian-keyring debian-archive-keyring apt-transport-https
# Chromium's browser sign-in (DICE) fails to get a token in Debian's build and then signs the notetaker's
# Google account out on every start; turning browser sign-in off keeps the plain web sign-in.
install -d -m 0755 /etc/chromium/policies/managed
echo '{"BrowserSignin": 0}' > /etc/chromium/policies/managed/recapz.json

if ! command -v caddy >/dev/null; then # the official Caddy apt repository
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -y && apt-get install -y caddy
fi

# Caddy's admin API goes on a unix socket (deploy/Caddyfile), not a localhost port any local process can reach
install -d /etc/systemd/system/caddy.service.d
printf '[Service]\nRuntimeDirectory=caddy\n' > /etc/systemd/system/caddy.service.d/10-recapz.conf

# no mDNS or LLMNR listeners; unattended upgrades reboot at 04:30 when a fix needs it
systemctl disable --now avahi-daemon.socket avahi-daemon.service 2>/dev/null || true
install -d /etc/systemd/resolved.conf.d
printf '[Resolve]\nLLMNR=no\nMulticastDNS=no\n' > /etc/systemd/resolved.conf.d/10-recapz.conf
if systemctl cat systemd-resolved >/dev/null 2>&1; then systemctl try-reload-or-restart systemd-resolved; fi
apt-get install -y unattended-upgrades
printf 'Unattended-Upgrade::Automatic-Reboot "true";\nUnattended-Upgrade::Automatic-Reboot-Time "04:30";\n' > /etc/apt/apt.conf.d/52-recapz-reboot

# under 4 GB RAM a call's Chromium needs room: 2 GB swap, used only under pressure
if [ "$(awk '/MemTotal/{print $2}' /proc/meminfo)" -lt 3500000 ] && ! swapon --show | grep -q /swapfile; then
  [ -e /swapfile ] || { fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile >/dev/null; }
  swapon /swapfile
  grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  echo vm.swappiness=10 > /etc/sysctl.d/99-recapz.conf && sysctl -p /etc/sysctl.d/99-recapz.conf >/dev/null
fi

id recapz >/dev/null 2>&1 || useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
id recapz-offsite >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin recapz-offsite
install -d -m 0750 -o root -g recapz /etc/recapz   # root owns config and keys; recapz only reads
install -d -m 0750 -o recapz -g recapz /var/backups/recapz   # recapz-offsite reads it via SupplementaryGroups=recapz
chmod 0750 /var/backups/recapz
for f in config.toml recapz.env; do [ ! -e /etc/recapz/$f ] || { chown root:recapz /etc/recapz/$f && chmod 0640 /etc/recapz/$f; }; done
install -d -m 0755 /opt/recapz/releases
install -m 0755 "$repo/scripts/backup.sh" /usr/local/bin/recapz-backup
install -m 0755 "$repo/scripts/offsite-backup.sh" /usr/local/bin/recapz-offsite
install -m 0755 "$repo/scripts/check-health.sh" /usr/local/bin/recapz-check-health
install -m 0755 "$repo/scripts/restore-test.sh" /usr/local/bin/recapz-restore-test
install -m 0755 "$repo/scripts/smoke-join.sh" /usr/local/bin/recapz-smoke-join
install -m 0644 "$repo"/deploy/recapz.service "$repo"/deploy/recapz-xvfb.service \
  "$repo"/deploy/recapz-backup.service "$repo"/deploy/recapz-backup.timer \
  "$repo"/deploy/recapz-offsite.service "$repo"/deploy/recapz-offsite.timer \
  "$repo"/deploy/recapz-health.service "$repo"/deploy/recapz-health.timer \
  "$repo"/deploy/recapz-restore-test.service "$repo"/deploy/recapz-restore-test.timer \
  "$repo"/deploy/recapz-smoke.service "$repo"/deploy/recapz-smoke.timer /etc/systemd/system/
systemctl daemon-reload
systemctl restart caddy   # picks up the drop-in (a reload cannot create the socket directory)

# firewall: SSH, HTTP (certificate challenge and redirect) and HTTPS only; the Xvfb display has no TCP listener
ufw default deny incoming && ufw default allow outgoing
ufw allow 22/tcp && ufw allow 80/tcp && ufw allow 443/tcp
ufw --force enable

# SSH: keys only (check you can log in with a key before running this)
if ! ls /root/.ssh/authorized_keys >/dev/null 2>&1 || [ ! -s /root/.ssh/authorized_keys ]; then
  echo "no root authorized_keys found: leaving SSH password login as it is" >&2
else
  printf 'PasswordAuthentication no\nPermitRootLogin prohibit-password\nAllowAgentForwarding no\nX11Forwarding no\nMaxAuthTries 3\nAllowTcpForwarding local\n' > /etc/ssh/sshd_config.d/10-recapz.conf
  sshd -t && systemctl reload ssh
fi

echo "host ready. Next: /etc/recapz/config.toml and recapz.env, the Caddyfile, deploy-release.sh, notetaker-login, then enable the units."
