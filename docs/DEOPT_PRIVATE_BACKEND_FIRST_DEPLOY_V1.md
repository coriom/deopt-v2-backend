# DeOpt private backend first deployment

The `DeOpt/production` Coolify Raw application (`dxco90jwpdxyennmquv2elyj`) was deployed once from Git commit `65115765e4e9e439c1663530fa69865d9eb21cda`, then redeployed once through Coolify to test restart without replacing its PostgreSQL volume. Deployment IDs: `fxsu1upif95c8zyhzrhu1upu` and `0l6mkyjcc01jexlhii39mow2`.

## Verified runtime

- The selected Compose source passed `docker compose config --format json` and `deploy/vps/check_coolify_private_compose.py`; 14 targeted validator tests passed. Coolify Raw has no generated preview, so the running containers were checked independently with `deploy/vps/check-private-production-runtime.py`.
- Operator-run root check returned `PRIVATE_PRODUCTION_RUNTIME=PASS` both before and after redeploy. The application has exactly one backend and one PostgreSQL 16 container. Their reviewed image digests, private network layout, secret mount, non-root read-only backend, resource limits and disabled economic flags passed.
- The same volume `dxco90jwpdxyennmquv2elyj_postgres_data` remained attached after redeploy. `_sqlx_migrations` contained 71 rows both times, matching 71 migration files in the repository.
- Container-local HTTP status: `/health` 200, `/ready` 200, missing markets 503, wallet history 403, unsupported write 405. Neither service publishes a host port. No domain or public API was configured.
- One idle `docker stats` sample after redeploy: backend 0.01% CPU and 9.199 MiB RAM of 2 GiB; PostgreSQL 0.08% CPU and 21.9 MiB RAM of 1.5 GiB. This is a point-in-time observation, not a capacity test.

## Backup and recovery boundary

`deploy/vps/backup-offsite.sh` performs a custom-format `pg_dump`, encrypts it with the reviewed public GPG recipient before transfer, uploads to R2 and compares the downloaded ciphertext hash. `deploy/vps/run-first-production-backup.sh` binds this to the exact production Compose project and PostgreSQL image, and stages only ciphertext in a root-owned export directory for transfer to WSL. `deploy/vps/operator-run-first-production-backup.sh` verifies the exact staged script hashes before installing root-owned copies and running the first backup. No private GPG key belongs on Netcup.

The operator ran the hash-checked root step because `deoptops` has no noninteractive sudo. The real dump was uploaded to `deopt_r2:deopt-private-backups/postgres/20261102T110240Z-502e9a2d5027b8c6dda858b444d3818b.dump.gpg` and retrieved with SHA-256 `32ee356c97844bc0b1f017de5a82a56f34049128472e9a0c3331058d4deb3085`. The independently copied WSL ciphertext is 48,512 bytes, mode 0600, with the same hash. The first R2 copy attempt received HTTP 501 `NotImplemented`; the second of three bounded retries succeeded. This is a successful upload/readback with a reliability warning, not a restore proof.

An isolated local PostgreSQL 16 target was used with no network or published ports. Default WSL GPG initially reported `No secret key`; the dedicated key was in `/home/corio/.local/share/deopt-backup-gpg`. A noninteractive attempt could not open pinentry, so the operator ran `deploy/vps/operator-restore-first-production-backup.sh` in their own WSL terminal with `GNUPGHOME` and `GPG_TTY` set. The script checked the ciphertext hash, performed a complete decryption preflight to `/dev/null`, then streamed the decrypted dump into `pg_restore --single-transaction --exit-on-error`. It returned `ISOLATED_POSTGRES_RESTORE=PASS` with 71 migrations and 93 public tables. Independent read-only checks found all 71 SQLx migration versions and SHA-384 checksums identical to the repository files, and zero rows in the other 92 tables. No production database restore was attempted; the active Coolify volume remained untouched.

`deploy/vps/run-daily-production-backup.sh` and the `deploy/vps/systemd/deopt-production-backup.{service,timer}` templates prepare a daily 02:00 UTC encrypted backup with a 15-minute randomized delay and a non-overlap lock. The runner pins the same Compose project, PostgreSQL image, root-owned backup script, public GPG keyring and root-only R2 configuration. It performs no retention deletion. The timer is **not installed or enabled**: an operator-owned failure notification destination must be selected, wired and tested before activation. `systemd-analyze calendar` accepted the schedule, and unit/timer syntax passed with an isolated staged executable. The production VPS configuration was not modified. Future application rollback must reselect the previous immutable image through Coolify after checking schema compatibility; it must not downgrade PostgreSQL or migrations.

## Scope

Base Sepolia chain ID 84532 only; indexer, trading, signing and broadcaster remain disabled. Wallet history is blocked. Frontend Vercel, Coolify proxy, SSH and blockchain governance were unchanged. This deployment validates private infrastructure and read-only application health; it does not authorize public API or trading.
