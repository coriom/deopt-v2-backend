# DeOpt Netcup/Coolify HTTPS release checkpoint

Checked 2026-11-03 UTC. This is a bounded release checkpoint, not an authorization to trade or broadcast.

## Current hosted state

| Gate | Status | Evidence / next requirement |
| --- | --- | --- |
| Coolify | PASS | v4.5.3 UI reachable through the private SSH tunnel; `DeOpt/production` resource `dxco90jwpdxyennmquv2elyj` displayed `Running:healthy`. |
| Backend private deployment | PASS in read-only scope | Previously verified root runtime: immutable GHCR image `sha256:b7ed384e9cae879579cf255fac79cce087de7f7c6922e65940c1147c78382436`, 71 migrations, `/health` and `/ready` 200, market 503, wallet history 403, write route 405. Image source commit `e2f139fc33286fea06d94989b31ab9d33aab8b98`; current `main` is newer. No backend redeployment was performed in this checkpoint. |
| PostgreSQL persistence | PASS for prior restart and restore evidence | Production PostgreSQL 16 uses a dedicated persistent Compose volume; a real R2 ciphertext was restored into isolated PostgreSQL, with 71 migrations and 93 public tables. No production database was restored or altered here. |
| Multi-project isolation | PASS for DeOpt configuration; capacity provisional | Coolify project/environment scope, dedicated secret and volume, PostgreSQL on `db` (`internal: true`), backend on `db` and `web`, no published DeOpt ports. Other applications must use separate networks, volumes, credentials and resource limits. A hosted load test is still needed before reserving capacity. |
| Public API HTTPS | BLOCKED | No approved API hostname/DNS or Coolify domain is configured. The wallet fee-history route lacks wallet authentication and must remain blocked. Do not publish it behind CORS alone. |
| Frontend integration | BLOCKED | The Vercel frontend uses `NEXT_PUBLIC_TRADING_API_BASE_URL`, but its `My Fees` client has no wallet-auth header. Keep the production setting unchanged until a protected HTTPS API and compatibility test exist. Read-only backend health does not imply trading availability. |
| Backup automation | PREPARED; not active | Discord-free daily systemd bundle and guarded installer are prepared. Operator root installation and a real scheduled execution/postflight remain. Existing one-time encrypted R2 backup and isolated restore passed earlier. |
| Proxy 8080 | PENDING runtime reconciliation | Coolify saved configuration now omits only `8080:8080`; the running socket still showed `0.0.0.0:8080` and `[::]:8080` at 09:12 UTC. External WSL `curl` failed to connect (HTTP 000), which does not prove a durable firewall policy. Do not claim the port closed. Original config SHA-256 `aa3249d26037668ff48725382da393fa8fb758baab671a344b7cc30df55cc670`; candidate SHA-256 `c9e17d1716197d8124b7ae0f686335f6ddb9574fcc324280768020b504986833`. The candidate passed `docker compose config`; its rendered diff removed only 8080. A controlled Coolify proxy restart and socket/80/443/tunnel postflight are still required; restore the original configuration if it fails. |

## Discord-free backup installation

`deploy/vps/systemd/no-discord/` removes the Discord `OnFailure` hook without changing the backup engine, daily 02:00 UTC timer, 15-minute jitter, persistence, or 36-hour freshness watchdog. `run-daily-production-backup.sh` retains the non-overlap lock and records success only after R2 readback. Journald and `systemctl --failed` retain local failure evidence. It does not automatically remove R2 objects.

The root installer `deploy/vps/operator-install-production-backup-local-v1.sh` checks the target hostname, pinned old unit and backup-engine hashes, and inactive timers; copies the staged files into a root-owned snapshot; checks the snapshot against a pinned manifest; backs up old units; installs and validates the new units; runs one genuine encrypted backup; and enables both timers only after the backup and freshness checks pass. Failure restores the old units and disables the new timers. The old Discord retry timer stays disabled. It requires one interactive sudo execution by the operator and has not run.

The nonsecret bundle was staged on Netcup at `/home/deoptops/deopt-backup-local.stage` (owner `deoptops`, mode 700). Remote `sha256sum --check` passed for all six manifest entries. The installer SHA-256 is `4e1ae962732766beff557a32245d546a7662cb9dc0f9922adf00a58f0a916ff2`; the manifest SHA-256 is `27d937991b02794da78ec8bfa5979659588d940b30f0fff6bc45240cdd32a9e2`. The operator must independently verify the installer hash and copy it root-owned before execution. No sudo command was run by Codex.

After installation, verify `systemctl is-active deopt-production-backup.timer deopt-production-backup-watchdog.timer`, `systemctl list-timers 'deopt-production-backup*'`, `systemctl show -p OnFailure deopt-production-backup.service deopt-production-backup-watchdog.service`, and the next real R2 object/readback. A full VPS outage needs external monitoring later.

## HTTPS and Vercel release order

1. Approve an actual API hostname and DNS target. Configure that name in Coolify and obtain a valid TLS certificate; do not use an invented temporary domain.
2. Implement and test wallet challenge/signature authentication, replay protection and authorization on the canonical fee-history API, plus rate limiting. Keep current 403 until this is accepted. Expose only explicitly reviewed routes through the proxy.
3. Test the hosted HTTPS API with anonymous denial, authenticated wallet isolation, `/health` and `/ready`, while confirming PostgreSQL stays unexposed and execution/trading flags remain false.
4. Add the frontend authentication contract, configure `NEXT_PUBLIC_TRADING_API_BASE_URL`, and run the real Vercel `My Fees` path. Trading remains disabled until separate protocol and operational authorization.

Rollback is image-by-digest through Coolify to the prior image; preserve the PostgreSQL volume and never automatically downgrade migrations. For the proxy, restore the saved original Compose and restart through Coolify, then check 80/443, private port 8000, other applications and DeOpt health. These are separate from economic rollback.

## Resource snapshot and limits

Netcup host snapshot: 7.8 GiB RAM total, 6.4 GiB available; 157 GiB root filesystem, 145 GiB available; no swap. The Compose caps PostgreSQL at 1.5 GiB and backend at 2 GiB. A prior idle sample showed backend 9.199 MiB and PostgreSQL 21.9 MiB, not a capacity benchmark. Reserve headroom for Coolify, its proxy and future projects; measure concurrent hosted load before adding more persistent applications.

## Focused checks

- `python3 -m unittest deploy.vps.test_production_backup_local deploy.vps.test_production_backup_alert -q`: 14 passed.
- `bash -n` on the installer and backup runner: passed.
- `systemd-analyze calendar '*-*-* 02:00:00 UTC'`: next 02:00 UTC shown.
- `systemd-analyze verify` with local executable stubs: passed; final verification must run on Netcup after root-owned installation.
- SSH: old installed units root:root:644 and backup/watchdog/Discord retry timers disabled.
- No backend, frontend, DNS, blockchain or public API deployment/change in this checkpoint.
