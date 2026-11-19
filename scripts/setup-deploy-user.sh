#!/usr/bin/env bash
# Usage: sudo setup-deploy-user.sh <public-key-file>     (once, on the host, as root)
# Creates the restricted `deploy` user for CI: its key can only run recapz-deploy-recv (see deploy-recv.sh):
# no shell, no port forwarding, no agent, no pty. Needs deploy-release.sh and deploy-recv.sh next to this file.
# Re-run it to replace the key.
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
pub=${1:?public key file}
here=$(cd "$(dirname "$0")" && pwd)
[ "$(wc -l <"$pub")" -le 1 ] && grep -q '^ssh-ed25519 ' "$pub" || { echo "$pub must hold one ssh-ed25519 public key" >&2; exit 1; }

install -m 0755 "$here/deploy-release.sh" /usr/local/sbin/recapz-deploy-release
install -m 0755 "$here/deploy-recv.sh" /usr/local/sbin/recapz-deploy-recv
id deploy >/dev/null 2>&1 || useradd --create-home --shell /bin/sh deploy
install -d -m 0700 -o deploy -g deploy /home/deploy/.ssh
printf 'restrict,command="sudo /usr/local/sbin/recapz-deploy-recv" %s\n' "$(cat "$pub")" >/home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys; chmod 0600 /home/deploy/.ssh/authorized_keys

tmp=$(mktemp); trap 'rm -f "$tmp"' EXIT
printf 'Defaults:deploy env_keep += "SSH_ORIGINAL_COMMAND"\ndeploy ALL=(root) NOPASSWD: /usr/local/sbin/recapz-deploy-recv\n' >"$tmp"
visudo -cf "$tmp" >/dev/null
install -m 0440 "$tmp" /etc/sudoers.d/recapz-deploy
echo "deploy user ready. Check from your machine: ssh -i <private key> deploy@<host> status"
