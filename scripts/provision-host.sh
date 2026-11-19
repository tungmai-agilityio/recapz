#!/usr/bin/env bash
# Usage: sudo scripts/provision-host.sh     (run from a checkout of this repo on a fresh Debian 12 host)
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
apt-get install -y chromium xvfb sqlite3 rsync curl ca-certificates gnupg ufw debian-keyring debian-archive-keyring apt-transport-https

if ! command -v caddy >/dev/null; then # the official Caddy apt repository
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -y && apt-get install -y caddy
fi

id recapz >/dev/null 2>&1 || useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
install -d -m 0750 -o recapz -g recapz /etc/recapz
install -d -m 0700 -o recapz -g recapz /var/backups/recapz
install -d -m 0755 /opt/recapz/releases
install -m 0755 "$repo/scripts/backup.sh" /usr/local/bin/recapz-backup
install -m 0644 "$repo"/deploy/recapz.service "$repo"/deploy/recapz-xvfb.service \
  "$repo"/deploy/recapz-backup.service "$repo"/deploy/recapz-backup.timer /etc/systemd/system/
systemctl daemon-reload

# firewall: SSH, HTTP (certificate challenge and redirect) and HTTPS only; the Xvfb display has no TCP listener
ufw default deny incoming && ufw default allow outgoing
ufw allow 22/tcp && ufw allow 80/tcp && ufw allow 443/tcp
ufw --force enable

# SSH: keys only (check you can log in with a key before running this)
if ! ls /root/.ssh/authorized_keys >/dev/null 2>&1 || [ ! -s /root/.ssh/authorized_keys ]; then
  echo "no root authorized_keys found: leaving SSH password login as it is" >&2
else
  printf 'PasswordAuthentication no\nPermitRootLogin prohibit-password\n' > /etc/ssh/sshd_config.d/10-recapz.conf
  systemctl reload ssh
fi

echo "host ready. Next: /etc/recapz/config.toml and recapz.env, the Caddyfile, deploy-release.sh, notetaker-login, then enable the units."
