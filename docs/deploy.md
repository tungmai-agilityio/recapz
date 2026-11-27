# Deploy (systemd)

**Status (2026-11-27):** running again on a new Droplet `recapz-dev` (Debian 13, 1 vCPU / 2 GB, 50 GB disk, Singapore `sgp1`, IP 143.198.199.104), sized for a one-person demo with 2 GB swap and `max_concurrent_calls = 1`. The `recapz` A record points at it (DNS only). Provisioned with `scripts/provision-host.sh`; release `v0.1.2-2fc71a3` installed by hand with `deploy-release.sh`; `scripts/smoke-deploy.sh` passed 6/6. The notetaker is signed in; no test Meet call has run on this host yet *(unverified)*. Recordings go to a private Cloudflare R2 bucket, untested with a real recording so far *(unverified)*. Offsite backups run daily to a separate R2 bucket (first snapshot and a test restore checked 2026-11-27). CI deploy is set for this host: the first CI deploy ran 2026-11-27 (`main-85f811d`, smoke test 6/6). Swap (2 GB) and `max_concurrent_calls = 1` are set because the host has 2 GB RAM. The previous Droplet (San Francisco) was deleted on 2026-11-24 with no data kept.

On a fresh Debian 12 or 13 host, `sudo scripts/provision-host.sh` does the packages, user, directories, unit files, firewall and key-only SSH below; then continue from the config and env files. Debian, not Ubuntu: Ubuntu's `chromium` is a snap that breaks under the service's hardening *(unverified here)*.

## Destroy and rebuild the Droplet

Destroying the Droplet saves cost (billing is hourly). What is lost and what to keep first:

| On the Droplet | Lost on destroy | Keep before destroying |
|---|---|---|
| `/var/lib/recapz` (per-tenant SQLite, recordings, uploads) | yes | `/var/backups/recapz` copied off the host (encrypted), or a snapshot (below) |
| `/etc/recapz/recapz.env` (`RECAPZ_SECRET_KEY`, speech, LLM and storage keys) | yes | the keys, in your password manager. Without the old `RECAPZ_SECRET_KEY` every member reconnects the calendar |
| `/etc/recapz/config.toml` (Google `client_id`/`client_secret`, `base_url`, `bootstrap_admin_email`) | yes | a private copy outside the repo |
| `notetaker-profile/` (the bot's Google sign-in) | yes, and never in backups | nothing: sign in again after the rebuild |
| Release history under `/opt/recapz/releases`, the SSH host key, the `deploy` user | yes | nothing: recreated below |

Before destroying, run both backups once (`sudo systemctl start recapz-backup.service recapz-offsite.service`) and check `journalctl -u recapz-offsite` saved a snapshot; without `/etc/recapz/offsite.env` ("Offsite" below) backups stay on the same disk and destroying loses all meetings.

### Optional: snapshot instead of a clean rebuild

A DigitalOcean snapshot keeps the whole disk, so the rebuild is only a new Droplet from the snapshot plus the DNS change. Power the Droplet off, take the snapshot (control panel: Droplet → Snapshots), then destroy it; snapshots are billed per GB per month (check DigitalOcean's current pricing). The snapshot keeps the data, config, env file, `deploy` user and SSH host key, so `DEPLOY_SSH_KEY` and `DEPLOY_KNOWN_HOSTS` stay valid. It also keeps the bot's session cookies and every secret, so treat it as sensitive, and delete it when no longer needed. After creating a Droplet from it: do step 2 (DNS), clear the old host key (`ssh-keygen -R recapz.tungmai.com`), `sudo systemctl start recapz`, and check that the bot is still signed in (Google may ask to sign in again from a new IP; if so, step 8). Steps 3 to 6 and 9 are not needed.

### Rebuild from scratch (clean Droplet)

**One command:** after step 1 (Droplet) and step 2 (DNS), put `config.toml`, `recapz.env` and optionally `offsite.env` in `.secrets/` at the repo root (git-ignored, mode 0700; `RECAPZ_SECRETS` picks another folder) and run `scripts/bootstrap-host.sh <ip>` from the repo root. It does steps 3 to 7 plus swap, the backups and the offsite timer, and can run again (a file missing in `.secrets/` but already on the host is kept; it deploys only when the site is not answering yet). It stops with a message if DNS does not point at the IP yet. It replaces the CI deploy key on every run. Still by hand: the Droplet, the A record, the Google redirect URIs, buckets and their keys, and step 8 (the notetaker's sign-in). Re-run on the live host 2026-11-27 with an empty secrets folder: all steps passed, the service kept running. A first run on a blank Droplet has not been done *(unverified)*. The steps below are what it runs.

Run on your machine unless a step says "on the host". Never paste a key into chat or put it on a command line; edit files on the server.

1. **Create the Droplet:** Debian 13, Singapore (`sgp1`, nearest the members), 1 vCPU / 2 GB for a demo (then add 2 GB swap and set `max_concurrent_calls = 1`) or 2 vCPU / 4 GB for a team; a CPU/RAM-only resize keeps the disk, so start small, your SSH key, enable the cloud firewall for 22, 80, 443. Note its IP.
2. **DNS:** in Cloudflare, set the `recapz` A record for `tungmai.com` to the new IP, **DNS only** (not proxied). Wait until `dig +short recapz.tungmai.com` shows it; Caddy cannot get a certificate before that. Then `ssh-keygen -R recapz.tungmai.com` to drop the old host key.
3. **Provision:** copy only the files the script needs, then run it on the host (it installs the packages, the Chromium policy `/etc/chromium/policies/managed/recapz.json` = `{"BrowserSignin": 0}`, the `recapz` user, directories, units, firewall and key-only SSH):
   ```bash
   HOST=recapz.tungmai.com; KEY=~/.ssh/recapz_do
   git archive HEAD scripts deploy | ssh -i $KEY root@$HOST 'mkdir -p /root/recapz && tar -x -C /root/recapz'
   ssh -i $KEY root@$HOST 'cd /root/recapz && ./scripts/provision-host.sh'
   ssh -i $KEY root@$HOST 'cat /etc/chromium/policies/managed/recapz.json'   # must print {"BrowserSignin": 0}
   ```
4. **Config and keys, on the host.** `config.toml` comes from `config.example.toml` with `data_dir = "/var/lib/recapz"`, `base_url = "https://recapz.tungmai.com"`, `dev_login = false`, `bootstrap_admin_email`, and the Google `client_id`/`client_secret` from your private copy. Put them in place and edit the env file in the editor, so no key is typed on a command line or sent through chat:
   ```bash
   scp -i $KEY config.example.toml root@$HOST:/etc/recapz/config.toml
   ssh -t -i $KEY root@$HOST 'install -m 0640 -o root -g recapz /root/recapz/deploy/recapz.env.example /etc/recapz/recapz.env && chown root:recapz /etc/recapz/config.toml && chmod 0640 /etc/recapz/config.toml && nano /etc/recapz/config.toml'
   ssh -t -i $KEY root@$HOST 'nano /etc/recapz/recapz.env'   # RECAPZ_SECRET_KEY (reuse the old one to keep calendar connections, or `openssl rand -hex 32`), RECAPZ_SPEECH_KEY, RECAPZ_LLM_KEY
   ```
   The service refuses to start without `RECAPZ_SPEECH_KEY`.
5. **Caddy:** on the host, `install -m 0644 /root/recapz/deploy/Caddyfile /etc/caddy/Caddyfile && systemctl restart caddy`. (`restart`, not `reload`, this once: the running Caddy still has its admin API on the old port, and only a restart moves it to the socket; after that `reload` works.) Check `https://recapz.tungmai.com` answers (502 is fine until step 7).
6. **CI access (once per new host):** do "Deploy from CI" step 1 (a new `ssh-keygen` for `~/recapz-ci-deploy`, since the old private key was deleted after it was stored in GitHub; `scp`, `setup-deploy-user.sh`, `status`). Then step 2, only the part that changes: `gh secret set DEPLOY_SSH_KEY --env production < ~/recapz-ci-deploy`, and re-set `DEPLOY_KNOWN_HOSTS` (the new host key; compare fingerprints first as described). `DEPLOY_HOST` and `DEPLOY_URL` stay the same because the domain is the same; the `production` environment, its reviewer and branch policies are kept by GitHub. The `status` check fails until the first release exists.
7. **First deploy:** only after steps 4 and 5, because a release that does not answer `/api/health` is rolled back and removed. Push a new tag name (an already pushed tag such as `v0.1.0` cannot be pushed again), or skip tagging: `gh workflow run release-build.yml --ref main -f deploy=true`. Follow "3. Release" below and press **Approve and deploy**. Then on the host: `systemctl enable --now recapz-xvfb recapz recapz-backup.timer` (the deploy restarts the service but nothing enables it at boot). The smoke test in the job must pass; run `./scripts/smoke-deploy.sh https://recapz.tungmai.com` again by hand if in doubt.
8. **Bot Google sign-in (VNC):** the notetaker has no saved sign-in on a new host. One command from your Mac: `scripts/host-notetaker-login.sh <ip>` opens the tunnel and VNC (localhost only, a one-time password it prints), opens Screen Sharing, runs `notetaker-login` as `recapz`; sign in, close the window, and it stops VNC and the tunnel and warns if a profile file is not owned by `recapz` (checked on the live host 2026-11-27). By hand instead: on the host install the VNC server once: `apt install x11vnc`. Then, from your machine:
   ```bash
   ssh -i $KEY -L 5900:localhost:5900 root@$HOST      # keep this open; on the host:
   x11vnc -display :99 -localhost -forever -passwd <choose-a-throwaway-password>
   ```
   Open `vnc://localhost:5900` (macOS Screen Sharing) and, in a second SSH session on the host, run `sudo -u recapz env DISPLAY=:99 recapz notetaker-login`. Sign in by hand in the window that appears, close it, stop `x11vnc` (Ctrl-C). Never run Chromium or `notetaker-login` as root: a root-owned file in `notetaker-profile/` breaks every call ("dropped before enough audio"). If one exists: `chown -R recapz:recapz /var/lib/recapz/notetaker-profile`. Google may challenge the sign-in from a new datacenter IP; that is not verified here.
9. **Restore data (if you kept a backup):** `systemctl stop recapz`, copy the stamp directory's contents over `/var/lib/recapz`, `chown -R recapz:recapz /var/lib/recapz`, but do not touch `notetaker-profile/`; start the service. Members keep their data; calendar connections work only with the old `RECAPZ_SECRET_KEY`.
10. **Check:** `./scripts/smoke-deploy.sh https://recapz.tungmai.com`, sign in as the admin, then a test Meet call (`scripts/smoke-join.sh`) to confirm the bot joins signed in. Set up "Offsite" under Backups and run it once.

## Install steps

Single host, one binary. `data_dir` in the config must be `/var/lib/recapz`.

```sh
make release
sudo install -m 0755 target/release/recapz /usr/local/bin/recapz
sudo useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
sudo install -d -m 0750 -o root -g recapz /etc/recapz
sudo install -m 0640 -o root -g recapz config.toml /etc/recapz/config.toml   # never commit config.toml
sudo install -m 0640 -o root -g recapz deploy/recapz.env.example /etc/recapz/recapz.env   # then fill in every key, see below
sudo install -m 0644 deploy/recapz.service deploy/recapz-xvfb.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-xvfb recapz
journalctl -u recapz -f
```

- **Cloud:** a DigitalOcean Droplet in Singapore, billed hourly (destroy it when unused; see Status above). It is a plain Linux VM, so everything below applies unchanged. DigitalOcean's Droplet backups and snapshots stay off (they would copy the notetaker's session cookies); the offsite restic backup covers the data. Debian 13 is what the host runs and what the rebuild steps assume; `scripts/provision-host.sh` was first written against Debian 12 (it only checks for Debian, not the version). A datacenter IP may be challenged by Meet more than a residential one *(unverified)*; the notetaker's saved sign-in and daily smoke test cover it.
- **Target host (hosted speech, `docs/decisions/hosted-speech.md`):** 2 vCPU, 4 GB RAM (the last Droplet had a 79 GB disk; about 40 GB is enough), `max_concurrent_calls = 2`, no GPU. Chromium (about 1 GB per call) drives memory; WAV audio (about 115 MB per hour) drives disk. On 2 GB RAM use `max_concurrent_calls = 1`. The speech and LLM keys go in `/etc/recapz/recapz.env` with `RECAPZ_SECRET_KEY`, never in `config.toml`.
- `make release` is a plain `cargo build --release`: no GPU or CUDA toolkit is needed to build or run (a C compiler is, for the TLS library).
- `bind` stays `127.0.0.1:8080`; Caddy terminates TLS in front of it: `sudo apt install caddy`, put `deploy/Caddyfile` at `/etc/caddy/Caddyfile` (the host is `recapz.tungmai.com`; change it if the domain changes; its DNS record must already point at the host), `sudo systemctl reload caddy`. Caddy also sends `Content-Security-Policy` (same-origin scripts, Google Fonts, `wss:` for live view and recording, `frame-ancestors 'none'`) and `X-Frame-Options DENY`; a new external origin the web app needs must be added to the CSP in `deploy/Caddyfile`, and a CSP violation shows in the browser console. Set `base_url` to the public https URL. Allow only ports 22, 80 and 443 in the host and cloud firewall. `scripts/provision-host.sh` also hardens the host: Caddy's admin API on `/run/caddy/admin.sock` (no local port), SSH with no agent, X11 or remote forwarding and `MaxAuthTries 3`, avahi and LLMNR off, and unattended upgrades rebooting at 04:30 when a kernel fix needs it (expect a short restart at night). DNS: `tungmai.com` is on Cloudflare with a DNS-only (not proxied) A record pointing at the Droplet; the IP is not recorded here.
- Set `dev_login = false` in production.
- Upgrade: get a Linux x86_64 `recapz` from the `release-build` workflow (Actions tab, Run workflow; then `gh run download <run-id> -n recapz-linux-x86_64`, and check `recapz.sha256`), or build it on a machine with the same OS and CPU (a 4 GB host needs about 4 GB of swap for the build, about 20 minutes, *measured only once*); `scp` it over, then `sudo scripts/deploy-release.sh ./recapz v0.4.0`. It installs the release under `/opt/recapz/releases/`, points `/usr/local/bin/recapz` at it, restarts the service and waits for `/api/health`; if the new release is not healthy it switches back and removes it. `sudo scripts/deploy-release.sh --rollback` goes back to the previous release by hand. Migrations are additive, so the older binary runs on the newer data. The first install is the same command (create `/opt/recapz/releases` first and skip the `install` of the binary in the setup block above).

## Meeting notetaker (Google Meet)

```sh
sudo apt install chromium xvfb          # or your distribution's Chromium; or set [notetaker] chromium_path
echo "RECAPZ_SECRET_KEY=$(openssl rand -hex 32)" | sudo tee -a /etc/recapz/recapz.env >/dev/null
# the Deepgram key: the server refuses to start without it (add it by editing the file, not on the command line)
sudoedit /etc/recapz/recapz.env   # add a line: RECAPZ_SPEECH_KEY=...
# recordings bucket (optional; the current host uses a private Cloudflare R2 bucket, see [storage] in config.toml): create a PRIVATE bucket, then add
#   RECAPZ_STORAGE_KEY_ID=...   and   RECAPZ_STORAGE_SECRET=...
```

- `RECAPZ_SECRET_KEY` encrypts stored Google refresh tokens. Keep a copy somewhere safe: losing it means every member reconnects the calendar. Never put it in `config.toml`.
- Add `<base_url>/auth/callback` (sign-in) and `<base_url>/auth/calendar/callback` as authorised redirect URIs in the Google client (Internal consent screen, `calendar.events.readonly`).
- The browser runs visible under the virtual display (`recapz-xvfb.service`, `DISPLAY=:99`), because a hidden browser is more likely to be challenged by the sign-in.
- Sign the notetaker's account in once, as the service user, on that display: reach it with SSH X forwarding or a VNC server pointed at `:99`, then `sudo -u recapz env DISPLAY=:99 recapz notetaker-login` (or run it with your own forwarded `DISPLAY`). Sign in by hand, close the window, then run a test call to confirm. `recapz notetaker-logout` deletes the saved sign-in.
- Browser sign-in must be off (`/etc/chromium/policies/managed/recapz.json` = `{"BrowserSignin": 0}`, written by `scripts/provision-host.sh`). Without it Chromium calls Google's Logout about a second after every start (`source=ChromiumAccountReconcilorDice`) and the notetaker joins Meet signed out. Add it before `notetaker-login`; on a host provisioned earlier, re-run `scripts/provision-host.sh`, then `notetaker-login`.
- Chromium's sandbox needs unprivileged user namespaces. If your distribution blocks them for the service user, allow them for Chromium (the browser is never started with the sandbox off).
- The saved sign-in (`notetaker-profile/`) and the per-call copies (`notetaker-tmp/`) hold live session cookies. They are excluded from backups; after a restore, run `notetaker-login` again.

## Workspaces

The server owner adds workspaces from the command line, on the host as the service user (the same config as the service):

```sh
sudo -u recapz recapz workspace-create "Acme" boss@example.com   # tenant, its database and an admin invite
sudo -u recapz recapz workspace-list                              # id, name, members, this month's cost (USD)
sudo -u recapz recapz workspace-cap <id> 25                       # this workspace's monthly cap in USD (0 = no cap)
```

`workspace-create` refuses a name already in use (any case). The admin joins by signing in with that Google account within 14 days; there is no self-service sign-up.

Cost caps: `monthly_cap_usd` in `config.toml` is the default cap of every workspace, `workspace-cap` gives one workspace its own, and `server_monthly_cap_usd` (0 = off) pauses paid jobs in every workspace once the month's total across workspaces reaches it. Workspace admins cannot change caps; `/api/usage` shows the workspace's effective cap.

## Backups

```sh
sudo install -m 0755 scripts/backup.sh /usr/local/bin/recapz-backup
sudo install -d -m 0750 -o recapz -g recapz /var/backups/recapz
sudo install -m 0644 deploy/recapz-backup.service deploy/recapz-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-backup.timer
```

Daily at 03:30 UTC; keeps 14 days. Stamps are `0750`/`0640` `recapz:recapz` (umask 027) so the offsite user can read them; stamps written before this change stay `0700`, so `sudo chmod -R g+rX /var/backups/recapz` once on an existing host. Needs `sqlite3` and `rsync` installed.
Restore: stop the service, copy a stamp directory's contents over `/var/lib/recapz`, fix ownership to `recapz`, start the service.

### Offsite (encrypted)

`recapz-offsite.timer` (04:00 UTC) pushes the newest stamp to an encrypted restic repository on any S3-compatible storage and keeps 7 daily and 4 weekly snapshots; the first run creates the repository. `scripts/provision-host.sh` installs restic, the script (`/usr/local/bin/recapz-offsite`) and the units. Use a private bucket of its own (not the recordings bucket) and, ideally, another provider than the host, so losing one account does not lose both. Create `/etc/recapz/offsite.env` (mode 0600, owner `root:root`: systemd reads it as root, so neither the app user nor the job user `recapz-offsite` can read the bucket key or the restic password; never commit it) by editing it on the host:

```sh
RESTIC_REPOSITORY=s3:https://<endpoint>/<bucket>
RESTIC_PASSWORD=<openssl rand -hex 32>   # also in your password manager: without it the backups cannot be read
AWS_ACCESS_KEY_ID=<bucket key id>
AWS_SECRET_ACCESS_KEY=<bucket secret>
```

The job runs as `recapz-offsite` (created by `scripts/provision-host.sh`, no shell, no home) with the supplementary group `recapz`, which is how it reads `/var/backups/recapz`. Code running as `recapz` cannot read the bucket key, the restic password or the offsite cache.

**Retention lock and pruning.** Turn on an R2 bucket lock rule on the offsite bucket: retain objects for at least 14 days (R2 dashboard, bucket Settings, Object lock). A key stolen from the host then cannot delete or overwrite snapshots younger than that. The job still runs `restic forget --prune` on the host (`scripts/offsite-backup.sh`), so prune can only remove packs older than the lock; a locked pack makes prune report an error until it ages out *(unverified against R2)*. If you want the host key unable to delete at all, use a bucket key without delete rights and run `restic forget --prune` yourself from a trusted machine with a separate key.

```sh
sudo systemctl enable --now recapz-offsite.timer
sudo systemctl start recapz-offsite.service && journalctl -u recapz-offsite -n 20   # run once now
```

Restore on a rebuilt host: `systemctl stop recapz`, then as root with the same env (`set -a; . /etc/recapz/offsite.env`) run `restic restore latest --target /var/lib/recapz`, `chown -R recapz:recapz /var/lib/recapz` and start the service. The notetaker's sign-in is never backed up; run `notetaker-login` again. `scripts/test-offsite.sh` checks backup, prune and restore against a local repository (needs restic; skipped without it).

## Health check and restore test

`scripts/check-health.sh` runs hourly and posts to `ALERT_WEBHOOK` (the same webhook and `/etc/recapz/smoke.env` as the smoke test) when a notetaker join failed in the last 70 minutes or a job has been locked or queued for over 60 minutes (`WINDOW_MINUTES`, `STUCK_MINUTES`). It reads each tenant database read-only; messages hold meeting and job IDs only. A join that stays failed is reported once per window, not once per hour. `scripts/restore-test.sh` runs monthly (1st, 04:30 UTC): it copies the newest backup stamp into a temp dir, checks every SQLite file opens and passes `integrity_check`, and posts the result (ok or failed). Neither script has been run on a real host *(unverified)*; `scripts/test-health.sh` covers them against a temp data dir.

```sh
sudo install -m 0755 scripts/check-health.sh /usr/local/bin/recapz-check-health
sudo install -m 0755 scripts/restore-test.sh /usr/local/bin/recapz-restore-test
sudo install -m 0644 deploy/recapz-health.* deploy/recapz-restore-test.* /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-health.timer recapz-restore-test.timer
sudo systemctl start recapz-restore-test.service   # run once now; check with journalctl -u recapz-restore-test
```

## Daily join smoke test

A timer sends the notetaker to a standing test call every day and alerts admins if the meeting does not reach `ready` (`scripts/smoke-join.sh`), so a Meet page change is caught before a team meeting is missed. Failures post to `ALERT_WEBHOOK`, the same webhook as the other alerts.

1. Keep one Meet call open for the test (a link that stays valid, e.g. a recurring event) and note its URL.
2. As a member (e.g. the admin) sign in on the web app, copy the `recapz_session` cookie value, and write `/etc/recapz/smoke.env` (mode 0600, owner `root:root`: systemd reads it for the units; never commit it):

```sh
RECAPZ_URL=http://localhost:8080
RECAPZ_COOKIE=recapz_session=<value>
MEET_TEST_URL=<the standing test call link>   # the test call link is set here
ALERT_WEBHOOK=<webhook url>
```

3. Install and enable:

```sh
sudo install -m 0755 scripts/smoke-join.sh /usr/local/bin/recapz-smoke-join
sudo install -m 0644 deploy/recapz-smoke.service deploy/recapz-smoke.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-smoke.timer
sudo systemctl start recapz-smoke.service   # run once now; check with journalctl -u recapz-smoke
```

Runs daily at 06:00 UTC. Someone must be in the test call (or it must admit the bot) when it runs. The session cookie expires with the session: when the script reports `join request failed`, refresh the cookie. Each run leaves a meeting in that member's account. This is **unverified** on a live host until the Meet live check (#424) is done.

## Deploy from CI

`release-build` builds the binary and then, after you approve, deploys it. The `deploy` job runs for a `v*` tag, or for a manual run on `main` with `deploy=true`; never on a plain merge. It sends the binary and its sha256 to a restricted `deploy` user on the host, runs `scripts/smoke-deploy.sh` against the live URL and rolls back if that fails (`deploy-release.sh` also rolls back by itself when the service does not become healthy). The CI key can only run `scripts/deploy-recv.sh`: `deploy <name> <sha256>` (binary on stdin), `rollback` and `status`; no shell, no forwarding. It can still ship any binary, so keep it only in the GitHub `production` environment.

### 1. One-time host setup

On your machine, in the repo root (`HOST` is the droplet; use your own `-i`/alias if you do not log in as root with this key):

```bash
HOST=recapz.tungmai.com
KEY=~/.ssh/recapz_do
```

Create the CI key (no passphrase, used only by GitHub):

```bash
ssh-keygen -t ed25519 -N '' -C recapz-ci-deploy -f ~/recapz-ci-deploy
```

Copy the scripts and the public key to the host:

```bash
scp -i $KEY scripts/deploy-release.sh scripts/deploy-recv.sh scripts/setup-deploy-user.sh ~/recapz-ci-deploy.pub root@$HOST:/root/
```

Install the `deploy` user (also installs both scripts under `/usr/local/sbin`, adds the `restrict` key and a sudoers entry for that one command; re-run it to replace the key):

```bash
ssh -i $KEY root@$HOST 'cd /root && ./setup-deploy-user.sh recapz-ci-deploy.pub'
```

Check it. This must print the release now in use, for example `/opt/recapz/releases/<name>/recapz`; any other command is refused:

```bash
ssh -i ~/recapz-ci-deploy deploy@$HOST status
```

### 2. One-time GitHub setup

Needs the `gh` CLI logged in as an admin of the repo; run from the repo root. Everything here can also be done in the web UI (repo **Settings → Environments**).

Create the `production` environment with you as required reviewer. Replace `<your-numeric-id>` with the output of `gh api user -q .id`, and limit deployments to `main` and `v*` tags:

```bash
REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
gh api -X PUT repos/$REPO/environments/production --input - <<EOF
{"reviewers":[{"type":"User","id":<your-numeric-id>}],
 "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF
gh api -X POST repos/$REPO/environments/production/deployment-branch-policies -f name=main -f type=branch
gh api -X POST repos/$REPO/environments/production/deployment-branch-policies -f name='v*' -f type=tag
```

Store the private key as the environment secret (read from the file, never printed or pasted into chat):

```bash
gh secret set DEPLOY_SSH_KEY --env production < ~/recapz-ci-deploy
```

Set the host, the URL and the pinned host key. First compare the fingerprints: the first command (on your machine) and the second (on the host) must print the same one, then store the scanned line:

```bash
ssh-keyscan -t ed25519 $HOST 2>/dev/null | ssh-keygen -lf -
ssh -i $KEY root@$HOST 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub'
gh variable set DEPLOY_HOST --env production --body $HOST
gh variable set DEPLOY_URL --env production --body https://recapz.tungmai.com
gh variable set DEPLOY_KNOWN_HOSTS --env production --body "$(ssh-keyscan -t ed25519 $HOST 2>/dev/null)"
```

Check what is set (values of secrets are never shown):

```bash
gh secret list --env production
gh variable list --env production
```

Optional: stop others from creating release tags. In the web UI: **Settings → Rules → Rulesets → New tag ruleset**, target pattern `v*`, restrict creations. Once the key is in GitHub you can delete it from your machine: `rm ~/recapz-ci-deploy`.

### 3. Release

Tag `main` and push the tag:

```bash
git switch main && git pull --ff-only
git tag v0.1.0
git push origin v0.1.0
```

Before approving, read what the tagged commit changes; anyone who can push a `v*` tag can run a workflow from that commit, and your approval is the only check. Look at `.github/` and `scripts/` first:

```bash
git diff --stat <previous-tag-or-live-release>..v0.1.0 -- .github scripts
```

Then find the run, wait for the build, and approve in the web UI (the run page → **Review deployments** → tick `production` → **Approve and deploy**):

```bash
gh run list --workflow release-build.yml --limit 3
gh run watch <run-id>
```

A deploy without a tag, from `main` (`make deploy` runs this and waits for the run; it fails when the build, deploy or smoke test fails):

```bash
gh workflow run release-build.yml --ref main -f deploy=true
```

Since 2026-11-27 the `production` environment has no required reviewer (the maintainer's choice), so these deploys start without **Approve and deploy**; add yourself as reviewer again to bring the gate back.

Check the result and, if needed, roll back by hand:

```bash
./scripts/smoke-deploy.sh https://recapz.tungmai.com
ssh -i ~/recapz-ci-deploy deploy@$HOST status
ssh -i ~/recapz-ci-deploy deploy@$HOST rollback
```

If the job fails: `DEPLOY_KEY is not set` (or another variable) means step 2 is incomplete; `Host key verification failed` means `DEPLOY_KNOWN_HOSTS` does not match the host; `Permission denied (publickey)` means the key in the secret is not the one installed in step 1; `refused: release ... is already live` means the tag name was already deployed, so use a new tag; `smoke test failed, rolling back` shows the failing check in the log and the previous release is back.

## After a deploy

`scripts/smoke-deploy.sh https://<domain>` checks health, the embedded web app, an asset's cache header, a client route, that the API refuses an unauthenticated call, and that http redirects to https. It needs only `curl`, no login and no provider calls. The Meet call check is separate: `scripts/smoke-join.sh`.

## macOS app (desktop)

The app is built on the maintainer's Mac, never in CI (macOS runners are billed 10x on the free plan), and attached to the GitHub release. It is signed with a self-signed "Recapz Local" certificate, so there is no notarization: members right-click the app → **Open** once. Keep signing with the same certificate: a new one counts as a different app and macOS asks again for the microphone and audio-capture grants.

One time on the Mac:

```bash
cargo install tauri-cli --version '^2' --locked
./scripts/desktop-cert.sh
```

The app updates itself (spec: `docs/specs/desktop-app.md`, "Auto-update"). Updates are signed with a separate key, created once on this Mac and never copied anywhere else (no CI, no repository, no chat). Keep it and its password in a password manager: without the key, installed apps can no longer be updated. `*.key` is git-ignored and the pre-commit hook refuses to commit a private key. Only the public key (`plugins.updater.pubkey` in `tauri.conf.json`) is committed.

```bash
cargo tauri signer generate -w ~/.tauri/recapz-updater.key
```

Each release, after `gh release create` (the version is the tag without the `v`):

```bash
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…'   # the key's password; the key itself is read from the file (UPDATER_KEY), nothing to paste
make desktop-dmg VERSION=0.1.3   # builds and signs; writes Recapz.dmg, Recapz.app.tar.gz, its .sig and latest.json
gh release upload v0.1.3 Recapz.dmg Recapz.app.tar.gz Recapz.app.tar.gz.sig
gh release upload v0.1.3 latest.json   # last: the app sees the update once this is up
```

**Which `VERSION`:** the tag of the release you just created, without the `v` (tag `v0.1.3` → `VERSION=0.1.3`). The release skill picks that number (step 3); `git tag --list 'v0.*' | sort -V | tail -1` shows the latest. The app only offers a build with a higher version than the installed one, and tags only go up, so every release qualifies. `tauri.conf.json` keeps `0.1.0` as a placeholder; the build overrides it. The first build with the updater is installed by hand once, and is just the next release's number, not a special one.

To try the updater without a release, run `make desktop-update-test`. It builds a 0.0.1 and a 0.0.2 (with plain http allowed in those two builds only), serves 0.0.2 on `localhost:8765` and prints what to check: the prompt, the restart as 0.0.2, the microphone and system-audio grants kept, and no `com.apple.quarantine` (`xattr -l`). It uses `~/recapz-update-test` and never touches `/Applications`. Testing a non-admin overwrite of `/Applications/Recapz.app` is a separate manual step.

Install the new build on your own Mac before uploading `latest.json`. The newest release must always have a `latest.json` (the app reads `releases/latest/download/latest.json`). When `desktop/` did not change, copy the previous one instead of building: `gh release download <previous tag> -p latest.json` then `gh release upload <tag> latest.json`; its URLs still point at the older assets. Members on a build from before the updater install the `.dmg` by hand once.

`make desktop-dmg` stops with a message if `cargo tauri` is not installed (the first command above). It bakes in `https://recapz.tungmai.com`; use `make desktop-dmg DESKTOP_BASE_URL=https://other.example` for another server. The updater reads `releases/latest/download/latest.json`, which does not exist until the first GitHub release carries desktop
assets, so until then the app's update check fails (logged only, nothing shown). A production build also needs the hosted
server back; with none running the app shows its "Can't reach" page (`TARGET=prod` builds were only checked with `make -n`).
Members install from the release page: open the `.dmg`, drag Recapz to Applications, then right-click → Open the first time (macOS 14.2 or later).
