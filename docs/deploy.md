# Deploy (systemd)

On a fresh Debian 12 host, `sudo scripts/provision-host.sh` does the packages, user, directories, unit files, firewall and key-only SSH below; then continue from the config and env files. Debian, not Ubuntu: Ubuntu's `chromium` is a snap that breaks under the service's hardening *(unverified here)*.

Single host, one binary. `data_dir` in the config must be `/var/lib/recapz`.

```sh
make release
sudo install -m 0755 target/release/recapz /usr/local/bin/recapz
sudo useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
sudo install -d -m 0750 -o recapz -g recapz /etc/recapz
sudo install -m 0640 -o recapz -g recapz config.toml /etc/recapz/config.toml   # never commit config.toml
sudo install -m 0640 -o recapz -g recapz deploy/recapz.env.example /etc/recapz/recapz.env   # then fill in every key, see below
sudo install -m 0644 deploy/recapz.service deploy/recapz-xvfb.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-xvfb recapz
journalctl -u recapz -f
```

- **Cloud:** a DigitalOcean Droplet in San Francisco, billed hourly (destroy it when unused). It is a plain Linux VM, so everything below applies unchanged. DigitalOcean's optional Droplet backups are not enabled, so copy `/var/backups/recapz` off the host. Debian 13 is what runs today; `scripts/provision-host.sh` and these docs were written against Debian 12 (the script only checks for Debian, not the version). A datacenter IP may be challenged by Meet more than a residential one *(unverified)*; the notetaker's saved sign-in and daily smoke test cover it.
- **Target host (hosted speech, `docs/decisions/hosted-speech.md`):** 2 vCPU, 4 GB RAM (the current Droplet has a 79 GB disk; about 40 GB is enough), `max_concurrent_calls = 2`, no GPU. Chromium (about 1 GB per call) drives memory; WAV audio (about 115 MB per hour) drives disk. On 2 GB RAM use `max_concurrent_calls = 1`. The speech and LLM keys go in `/etc/recapz/recapz.env` with `RECAPZ_SECRET_KEY`, never in `config.toml`.
- `make release` is a plain `cargo build --release`: no GPU or CUDA toolkit is needed to build or run (a C compiler is, for the TLS library).
- `bind` stays `127.0.0.1:8080`; Caddy terminates TLS in front of it: `sudo apt install caddy`, put `deploy/Caddyfile` at `/etc/caddy/Caddyfile` (the host is `recapz.tungmai.com`; change it if the domain changes; its DNS record must already point at the host), `sudo systemctl reload caddy`. Set `base_url` to the public https URL. Allow only ports 22, 80 and 443 in the host and cloud firewall. DNS: `tungmai.com` is on Cloudflare with a DNS-only (not proxied) A record pointing at the Droplet; the IP is not recorded here.
- Set `dev_login = false` in production.
- Upgrade: get a Linux x86_64 `recapz` from the `release-build` workflow (Actions tab, Run workflow; then `gh run download <run-id> -n recapz-linux-x86_64`, and check `recapz.sha256`), or build it on a machine with the same OS and CPU (a 4 GB host needs about 4 GB of swap for the build, about 20 minutes, *measured only once*); `scp` it over, then `sudo scripts/deploy-release.sh ./recapz v0.4.0`. It installs the release under `/opt/recapz/releases/`, points `/usr/local/bin/recapz` at it, restarts the service and waits for `/api/health`; if the new release is not healthy it switches back and removes it. `sudo scripts/deploy-release.sh --rollback` goes back to the previous release by hand. Migrations are additive, so the older binary runs on the newer data. The first install is the same command (create `/opt/recapz/releases` first and skip the `install` of the binary in the setup block above).

## Meeting notetaker (Google Meet)

```sh
sudo apt install chromium xvfb          # or your distribution's Chromium; or set [notetaker] chromium_path
echo "RECAPZ_SECRET_KEY=$(openssl rand -hex 32)" | sudo tee -a /etc/recapz/recapz.env >/dev/null
# the Deepgram key: the server refuses to start without it (add it by editing the file, not on the command line)
sudoedit /etc/recapz/recapz.env   # add a line: RECAPZ_SPEECH_KEY=...
# recordings bucket (optional, not enabled today: recordings stay on server disk; the object storage provider is undecided; see [storage] in config.toml): create a PRIVATE bucket, then add
#   RECAPZ_STORAGE_KEY_ID=...   and   RECAPZ_STORAGE_SECRET=...
```

- `RECAPZ_SECRET_KEY` encrypts stored Google refresh tokens. Keep a copy somewhere safe: losing it means every member reconnects the calendar. Never put it in `config.toml`.
- Add `<base_url>/auth/calendar/callback` as an authorised redirect URI in the Google client (Internal consent screen, `calendar.events.readonly`).
- The browser runs visible under the virtual display (`recapz-xvfb.service`, `DISPLAY=:99`), because a hidden browser is more likely to be challenged by the sign-in.
- Sign the notetaker's account in once, as the service user, on that display: reach it with SSH X forwarding or a VNC server pointed at `:99`, then `sudo -u recapz env DISPLAY=:99 recapz notetaker-login` (or run it with your own forwarded `DISPLAY`). Sign in by hand, close the window, then run a test call to confirm. `recapz notetaker-logout` deletes the saved sign-in.
- Chromium's sandbox needs unprivileged user namespaces. If your distribution blocks them for the service user, allow them for Chromium (the browser is never started with the sandbox off).
- The saved sign-in (`notetaker-profile/`) and the per-call copies (`notetaker-tmp/`) hold live session cookies. They are excluded from backups; after a restore, run `notetaker-login` again.

## Backups

```sh
sudo install -m 0755 scripts/backup.sh /usr/local/bin/recapz-backup
sudo install -d -m 0700 -o recapz -g recapz /var/backups/recapz
sudo install -m 0644 deploy/recapz-backup.service deploy/recapz-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-backup.timer
```

Daily at 03:30 UTC; keeps 14 days. Needs `sqlite3` and `rsync` installed. Copy `/var/backups/recapz` off the host (it contains real meeting data, so encrypt it in transit and at rest).
Restore: stop the service, copy a stamp directory's contents over `/var/lib/recapz`, fix ownership to `recapz`, start the service.

## Deploy key for CI

CI deploys with its own SSH key, never your personal one. On your machine: `ssh-keygen -t ed25519 -N '' -C recapz-ci-deploy -f ~/recapz-ci-deploy` (this stays on your machine until it goes into a GitHub secret). Copy `scripts/deploy-release.sh`, `scripts/deploy-recv.sh`, `scripts/setup-deploy-user.sh` and `~/recapz-ci-deploy.pub` to the host and run `sudo ./setup-deploy-user.sh recapz-ci-deploy.pub`. That creates the `deploy` user whose key can run only `scripts/deploy-recv.sh`: `deploy <name> <sha256>` (binary on stdin, checked, then installed by `deploy-release.sh`), `rollback` and `status`. No shell, no forwarding. A stolen key can still ship any binary, so keep it only in the GitHub `production` environment and replace it by re-running the script. Check it with `ssh -i ~/recapz-ci-deploy deploy@<host> status`.

## Deploy from CI

`release-build` deploys through the `deploy` job after you approve it. It runs for a `v*` tag, or for a manual run on `main` with "deploy" ticked; never on a plain merge. The job sends the binary and its sha256 to the `deploy` user (see "Deploy key for CI"), then runs `scripts/smoke-deploy.sh` against the live URL and rolls back if it fails. `deploy-release.sh` also rolls back by itself if the service is not healthy. One-time setup (GitHub, repo **Settings → Environments**):

1. **New environment**, name `production`. Tick **Required reviewers**, add yourself, **Save protection rules**. Under **Deployment branches and tags** choose **Selected branches and tags**, add the tag rule `v*` and the branch `main`.
2. In that environment add the secret `DEPLOY_SSH_KEY`: the full contents of `~/recapz-ci-deploy` (`pbcopy < ~/recapz-ci-deploy`, paste in the box; never in chat or a file in the repo). Then delete the private key from your machine if you do not need it elsewhere.
3. Add the environment variables `DEPLOY_HOST` (`159.223.198.251`), `DEPLOY_URL` (`https://recapz.tungmai.com`) and `DEPLOY_KNOWN_HOSTS`, the pinned host key. Get it from a machine you trust and compare the fingerprint with the one the host reports over your own login: `ssh-keyscan -t ed25519 159.223.198.251` on your machine; on the host `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`; `ssh-keygen -lf -` on the scanned line must print the same fingerprint. Paste the scanned line (`159.223.198.251 ssh-ed25519 AAAA...`) as the value.
4. Before you approve, open the tagged commit and read what it changes, `.github/` and `scripts/` first: anyone who can push a `v*` tag can run a workflow from that commit, and your approval is the only check. Add a tag protection rule for `v*` (Settings → Rules → Rulesets) so only you can create them.
5. Release: push a tag (`git tag v0.5.0 && git push origin v0.5.0`), open the run, **Review deployments**, tick `production`, **Approve and deploy**. A manual run needs **Run workflow** on `main` with "deploy" ticked.

## After a deploy

`scripts/smoke-deploy.sh https://<domain>` checks health, the embedded web app, an asset's cache header, a client route, that the API refuses an unauthenticated call, and that http redirects to https. It needs only `curl`, no login and no provider calls. The Meet call check is separate: `scripts/smoke-join.sh`.
