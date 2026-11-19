# Deploy (systemd)

Single host, one binary. `data_dir` in the config must be `/var/lib/recapz`.

```sh
make release
sudo install -m 0755 target/release/recapz /usr/local/bin/recapz
sudo useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
sudo install -d -m 0750 -o recapz -g recapz /etc/recapz
sudo install -m 0640 -o recapz -g recapz config.toml /etc/recapz/config.toml   # never commit config.toml
sudo install -m 0640 -o recapz -g recapz /dev/null /etc/recapz/recapz.env       # then add RECAPZ_SECRET_KEY, RECAPZ_SPEECH_KEY and the storage keys, see below
sudo install -m 0644 deploy/recapz.service deploy/recapz-xvfb.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-xvfb recapz
journalctl -u recapz -f
```

- **Cloud:** Hetzner Cloud, Singapore region (decided 2026-11-18): cheapest for this size and closest to a Vietnam team. It is a plain Linux VM, so everything below applies unchanged. Backups are not included by default; copy `/var/backups/recapz` off the host. A datacenter IP may be challenged by Meet more than a residential one *(unverified)*; the notetaker's saved sign-in and daily smoke test cover it.
- **Target host (hosted speech, `docs/decisions/hosted-speech.md`):** 2 vCPU, 4 GB RAM, 40 GB disk, `max_concurrent_calls = 2`, no GPU. Chromium (about 1 GB per call) drives memory; WAV audio (about 115 MB per hour) drives disk. On 2 GB RAM use `max_concurrent_calls = 1`. The speech and LLM keys go in `/etc/recapz/recapz.env` with `RECAPZ_SECRET_KEY`, never in `config.toml`.
- `make release` is a plain `cargo build --release`: no GPU or CUDA toolkit is needed to build or run (a C compiler is, for the TLS library).
- `bind` stays `127.0.0.1:8080`; put a TLS reverse proxy in front. Set `base_url` to the public https URL.
- Set `dev_login = false` in production.
- Upgrade: install the new binary, `sudo systemctl restart recapz` (migrations run on start and are additive).

## Meeting notetaker (Google Meet)

```sh
sudo apt install chromium xvfb          # or your distribution's Chromium; or set [notetaker] chromium_path
echo "RECAPZ_SECRET_KEY=$(openssl rand -hex 32)" | sudo tee -a /etc/recapz/recapz.env >/dev/null
# the Deepgram key: the server refuses to start without it (add it by editing the file, not on the command line)
sudoedit /etc/recapz/recapz.env   # add a line: RECAPZ_SPEECH_KEY=...
# recordings bucket (optional; see [storage] in config.toml): create a PRIVATE bucket, then add
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

## After a deploy

`scripts/smoke-deploy.sh https://<domain>` checks health, the embedded web app, an asset's cache header, a client route, that the API refuses an unauthenticated call, and that http redirects to https. It needs only `curl`, no login and no provider calls. The Meet call check is separate: `scripts/smoke-join.sh`.
