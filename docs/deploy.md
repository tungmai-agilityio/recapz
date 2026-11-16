# Deploy (systemd)

Single host, one binary. `data_dir` in the config must be `/var/lib/recapz`.

```sh
make release
sudo install -m 0755 target/release/recapz /usr/local/bin/recapz
sudo useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
sudo install -d -m 0750 -o recapz -g recapz /etc/recapz
sudo install -m 0640 -o recapz -g recapz config.toml /etc/recapz/config.toml   # never commit config.toml
sudo install -m 0640 -o recapz -g recapz /dev/null /etc/recapz/recapz.env       # then add RECAPZ_SECRET_KEY, see below
sudo install -m 0644 deploy/recapz.service deploy/recapz-xvfb.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-xvfb recapz
journalctl -u recapz -f
```

- `bind` stays `127.0.0.1:8080`; put a TLS reverse proxy in front. Set `base_url` to the public https URL.
- Set `dev_login = false` in production.
- Upgrade: install the new binary, `sudo systemctl restart recapz` (migrations run on start and are additive).

## HTTPS with Cloudflare Tunnel (instead of a reverse proxy)

The tunnel makes an outbound connection from the host, so no inbound port is opened and `bind` stays `127.0.0.1:8080`. The domain must be on Cloudflare DNS.

```sh
# install cloudflared from Cloudflare's package repository, then:
cloudflared tunnel login
cloudflared tunnel create recapz
cloudflared tunnel route dns recapz recapz.example.com
sudo install -d /etc/cloudflared
sudo tee /etc/cloudflared/config.yml >/dev/null <<'YAML'
tunnel: recapz
credentials-file: /etc/cloudflared/<tunnel-id>.json
ingress:
  - hostname: recapz.example.com
    service: http://127.0.0.1:8080
  - service: http_status:404
YAML
sudo cp ~/.cloudflared/<tunnel-id>.json /etc/cloudflared/ && sudo chmod 0600 /etc/cloudflared/<tunnel-id>.json
sudo cloudflared service install && sudo systemctl enable --now cloudflared
```

- Set `base_url = "https://recapz.example.com"` and use the same host in the Google redirect URI.
- Cloudflare's proxy caps a request body at 100 MB on the Free and Pro plans, so larger uploads fail through the tunnel. Check the largest upload you expect before relying on it.
- The tunnel credentials file lets anyone run your tunnel: keep it `0600` and out of git.

## Meeting notetaker (Google Meet)

```sh
sudo apt install chromium xvfb          # or your distribution's Chromium; or set [notetaker] chromium_path
echo "RECAPZ_SECRET_KEY=$(openssl rand -hex 32)" | sudo tee -a /etc/recapz/recapz.env >/dev/null
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

### Off-host copy to Cloudflare R2

`backup.sh` keeps the last 14 days on the host's own disk. Copy each run to a private R2 bucket so a lost disk does not lose the backups.

```sh
sudo apt install rclone
sudo -u recapz rclone config create r2 s3 provider Cloudflare \
  access_key_id <key> secret_access_key <secret> endpoint https://<account-id>.r2.cloudflarestorage.com
sudo systemctl edit recapz-backup.service      # add the two lines below
```

```ini
[Service]
ExecStartPost=/usr/bin/rclone copy /var/backups/recapz r2:recapz-backups
```

- Create the bucket private, with an R2 API token scoped to that one bucket (Object Read & Write). The token lives in `~recapz/.config/rclone/rclone.conf`; keep it `0600`.
- Use `copy`, not `sync`: `sync` would delete the remote copies when the local ones age out, or when the local directory is lost.
- Expire old objects with an R2 lifecycle rule on the bucket (for example 30 days) instead of deleting from the host.
- R2 encrypts at rest and rclone uses TLS in transit. For a key only you hold, wrap the remote with `rclone config` type `crypt`; losing that password makes the backups unreadable.
- Restore: `rclone copy r2:recapz-backups/<stamp> /var/backups/recapz/<stamp>`, then follow the steps above.
