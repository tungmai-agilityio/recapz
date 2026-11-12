# Deploy (systemd)

Single host, one binary. `data_dir` in the config must be `/var/lib/recapz`.

```sh
make release
sudo install -m 0755 target/release/recapz /usr/local/bin/recapz
sudo useradd --system --home /var/lib/recapz --create-home --shell /usr/sbin/nologin recapz
sudo install -d -m 0750 -o recapz -g recapz /etc/recapz
sudo install -m 0640 -o recapz -g recapz config.toml /etc/recapz/config.toml   # never commit config.toml
sudo install -m 0644 deploy/recapz.service /etc/systemd/system/recapz.service
sudo systemctl daemon-reload && sudo systemctl enable --now recapz
journalctl -u recapz -f
```

- `bind` stays `127.0.0.1:8080`; put a TLS reverse proxy in front. Set `base_url` to the public https URL.
- Set `dev_login = false` in production.
- Upgrade: install the new binary, `sudo systemctl restart recapz` (migrations run on start and are additive).


## Backups

```sh
sudo install -m 0755 scripts/backup.sh /usr/local/bin/recapz-backup
sudo install -d -m 0700 -o recapz -g recapz /var/backups/recapz
sudo install -m 0644 deploy/recapz-backup.service deploy/recapz-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now recapz-backup.timer
```

Daily at 03:30 UTC; keeps 14 days. Needs `sqlite3` and `rsync` installed. Copy `/var/backups/recapz` off the host (it contains real meeting data, so encrypt it in transit and at rest).
Restore: stop the service, copy a stamp directory's contents over `/var/lib/recapz`, fix ownership to `recapz`, start the service.
