# DeOpt production PostgreSQL backup automation

The existing private Coolify application and PostgreSQL volume are unchanged. The existing encrypted `backup-offsite.sh` remains the backup engine: it dumps only the reviewed production PostgreSQL container, encrypts before R2 upload, and verifies downloaded ciphertext. No R2 retention deletion is configured.

## Behavior

- `deopt-production-backup.timer`: 02:00 UTC daily, up to 15 minutes of random delay, `AccuracySec=1us`, `Persistent=true`. The runner uses an exclusive lock and records `/var/lib/deopt-backup/last-success` **only after** R2 readback hash verification.
- `deopt-production-backup.service`: on failure, systemd starts `deopt-backup-alert@%n.service`. The notifier reads the Discord URL through `LoadCredential=` from `/etc/deopt/backup-discord-webhook`; the URL is not an argument, environment variable, log line or repository value.
- Discord content has exactly four lines: service name, UTC time, `FAIL`, and a nonsecret execution ID. `allowed_mentions` is empty. The notifier uses `wait=true` and requires a returned message ID. It retries transient errors three times and stores an undelivered minimal alert in a root-only queue. `deopt-backup-alert-retry.timer` retries up to five queued alerts every 15 minutes. An ambiguous network response can produce a duplicate notification; delivery is at least once, not exactly once.
- `deopt-production-backup-watchdog.timer`: hourly check of the last verified success. Missing, malformed, future-dated or older than 36 hours is a failed check, triggering the same Discord alert. The notifier sends at most one watchdog alert per unchanged stale backup state. A later successful backup clears that incident marker.
- A full VPS outage cannot send Discord notifications from this host. An independent external uptime/backup monitor is required for that failure mode.

## Single operator installation

The nonsecret bundle is staged at `/home/deoptops/deopt-backup-automation.stage/`. The installer checks the fixed manifest SHA-256 `7f9b11ce834c38318e66de02e6f6259769eff54d155cb156ff7f726396d6495c`, every bundle file, all root-owned copies against the manifest, and the previously verified backup script. Its own SHA-256 is `54147fcd2fdb87f45d4858810f9211370abd25b5b280ea926a8792a56b853938`. It refuses an existing conflicting unit or an already active timer.

In the operator's Netcup terminal, copy the reviewed installer root-owned after checking its SHA-256, then run it with sudo. It prompts for the Discord webhook URL with hidden input and writes `/etc/deopt/backup-discord-webhook` root:root mode 0600 in a root:root mode 0700 directory. Never paste the URL into chat, Git, a shell command or a log.

```bash
sudo bash -c '
set -euo pipefail
source_file=/home/deoptops/deopt-backup-automation.stage/operator-install-production-backup-automation.sh
root_file=/root/deopt-install-production-backup-automation-v1.sh
expected=54147fcd2fdb87f45d4858810f9211370abd25b5b280ea926a8792a56b853938
test "$(sha256sum "$source_file" | cut -d " " -f 1)" = "$expected"
install -o root -g root -m 0700 "$source_file" "$root_file"
test "$(sha256sum "$root_file" | cut -d " " -f 1)" = "$expected"
bash "$root_file"
'
```

The installer verifies systemd units, intentionally fails **only** `deopt-backup-failure-probe.service`, and waits for a Discord message created by its `OnFailure` handler. The operator types `RECEIVED` only after seeing that message. It then starts one genuine successful encrypted backup, verifies the 36-hour freshness check, and enables the backup, watchdog and alert-retry timers. On any error, it disables those three timers without removing PostgreSQL, R2 objects, secrets or evidence.

After installation, verify `systemctl is-active` and `systemctl is-enabled` for all three timers, plus `NextElapseUSecRealtime` for the backup timer. Do not declare activation from local unit tests alone.

## Rollback

`sudo systemctl disable --now deopt-production-backup.timer deopt-production-backup-watchdog.timer deopt-backup-alert-retry.timer`

This stops only future automation. It does not stop DeOpt, delete R2 objects, alter the database or remove the root-only webhook. Retain the secret for controlled diagnosis or remove it later under a separate reviewed action.

References: [systemd timer semantics](https://www.freedesktop.org/software/systemd/man/latest/systemd.timer.html), [systemd OnFailure semantics](https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html), [systemd credentials](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html), [Discord webhook execution](https://github.com/discord/discord-api-docs/blob/main/developers/resources/webhook.mdx).
