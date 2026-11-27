#!/usr/bin/env bash
# Usage: scripts/bootstrap-host.sh <droplet-ip>     (on your machine, from the repo root, with gh logged in)
# Sets up a fresh Debian 13 host end to end: provision, config and env files, Caddy, the CI deploy user and
# DEPLOY_* settings, the units, a first deploy of main, and one run of both backups. Every step can run again.
#
# Secrets are read from RECAPZ_SECRETS (default .secrets/ in the repo root: git-ignored, keep it 0700):
#   config.toml   your config (data_dir, base_url, dev_login and, under 4 GB RAM, max_concurrent_calls are set here)
#   recapz.env    RECAPZ_SECRET_KEY, RECAPZ_SPEECH_KEY, RECAPZ_LLM_KEY, ... (deploy/recapz.env.example)
#   offsite.env   optional, restic settings (docs/deploy.md, "Offsite")
# A file missing locally but already on the host is kept. Other settings: RECAPZ_DOMAIN (recapz.tungmai.com),
# RECAPZ_SSH_KEY (~/.ssh/recapz_do). Still by hand: the Droplet, the DNS A record, Google redirect URIs, buckets
# and their keys, and the notetaker's Google sign-in.
set -euo pipefail
ip=${1:?usage: bootstrap-host.sh <droplet-ip>}
domain=${RECAPZ_DOMAIN:-recapz.tungmai.com}
secrets=${RECAPZ_SECRETS:-.secrets}
key=${RECAPZ_SSH_KEY:-$HOME/.ssh/recapz_do}
host=(ssh -i "$key" -o BatchMode=yes -o StrictHostKeyChecking=accept-new "root@$ip")
step() { echo "== $*"; }
# gh must work before the host's CI key is replaced, or the host and GitHub end up with different keys
gh auth status >/dev/null 2>&1 || { echo "gh is not logged in (gh auth login)" >&2; exit 1; }

step "provision $ip"
git archive HEAD scripts deploy | "${host[@]}" 'rm -rf /root/recapz && mkdir -p /root/recapz && tar -x -C /root/recapz &&
  { /root/recapz/scripts/provision-host.sh >/root/provision.log 2>&1 || { tail -20 /root/provision.log; exit 1; }; }'

step "config and env files"
pushed=0
for f in config.toml recapz.env offsite.env; do
  if [ -f "$secrets/$f" ]; then # written through stdin: no secret on a command line
    "${host[@]}" "umask 077 && cat > /etc/recapz/$f.new && chown recapz:recapz /etc/recapz/$f.new && chmod 0640 /etc/recapz/$f.new && mv /etc/recapz/$f.new /etc/recapz/$f" < "$secrets/$f"
    pushed=1
  elif [ "$f" != offsite.env ] && ! "${host[@]}" "test -s /etc/recapz/$f"; then
    echo "missing $secrets/$f (and none on the host)" >&2; exit 1
  fi
done
"${host[@]}" "f=/etc/recapz/config.toml
  sed -i -e '0,/^data_dir = .*/s||data_dir = \"/var/lib/recapz\"|' -e '0,/^base_url = .*/s||base_url = \"https://$domain\"|' -e 's|^dev_login = true|dev_login = false|' \$f
  [ \$(awk '/MemTotal/{print \$2}' /proc/meminfo) -ge 3500000 ] || sed -i 's|^max_concurrent_calls = [0-9]*|max_concurrent_calls = 1|' \$f
  grep -q '^RECAPZ_SPEECH_KEY=.' /etc/recapz/recapz.env || { echo 'RECAPZ_SPEECH_KEY is empty in recapz.env' >&2; exit 1; }"

step "DNS and Caddy"
[ "$(dig +short "$domain" | tail -1)" = "$ip" ] || { echo "point the $domain A record (DNS only) at $ip, then run this again" >&2; exit 1; }
"${host[@]}" "sed 's/recapz\.tungmai\.com/$domain/' /root/recapz/deploy/Caddyfile > /etc/caddy/Caddyfile && systemctl reload caddy"

step "CI deploy user and GitHub settings"
ssh-keygen -R "$domain" >/dev/null 2>&1 || true
known=$(ssh-keyscan -t ed25519 "$domain" 2>/dev/null)
[ -n "$known" ] || { echo "no host key from $domain" >&2; exit 1; }
[ "$(ssh-keygen -lf - <<<"$known" | awk '{print $2}')" = "$("${host[@]}" 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub' | awk '{print $2}')" ] ||
  { echo "host key of $domain does not match $ip" >&2; exit 1; }
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
ssh-keygen -q -t ed25519 -N '' -C recapz-ci-deploy -f "$tmp/ci"
"${host[@]}" 'cat > /root/recapz/ci.pub && /root/recapz/scripts/setup-deploy-user.sh /root/recapz/ci.pub >/dev/null' < "$tmp/ci.pub"
gh secret set DEPLOY_SSH_KEY --env production < "$tmp/ci"
gh variable set DEPLOY_HOST --env production --body "$domain"
gh variable set DEPLOY_URL --env production --body "https://$domain"
gh variable set DEPLOY_KNOWN_HOSTS --env production --body "$known"

step "units"
"${host[@]}" 'set -e; systemctl enable -q recapz; systemctl enable -q --now recapz-xvfb recapz-backup.timer
  if [ -s /etc/recapz/offsite.env ]; then systemctl enable -q --now recapz-offsite.timer; fi'
# a pushed config or env file only takes effect after a restart (this ends a call in progress)
[ "$pushed" = 0 ] || "${host[@]}" "systemctl is-active -q recapz && systemctl restart recapz || true"

step "deploy"
if curl -fsS -m 10 "https://$domain/api/health" >/dev/null 2>&1; then echo "already running; deploy new code with make deploy"; else make deploy; fi

step "backups"
"${host[@]}" 'systemctl start recapz-backup.service && if [ -s /etc/recapz/offsite.env ]; then systemctl start recapz-offsite.service; fi'
echo "done. Next: the notetaker's Google sign-in (docs/deploy.md, step 8), then a test Meet call."
