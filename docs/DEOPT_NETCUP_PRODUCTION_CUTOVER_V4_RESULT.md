# DEOPT_NETCUP_PRODUCTION_CUTOVER_V4_RESULT

Executed on 2026-11-04, Base Sepolia private read-only release. Coolify resource and Compose project: `dxco90jwpdxyennmquv2elyj`. No public backend domain or port was configured.

| Gate | Verdict | Hosted evidence |
| --- | --- | --- |
| PRE_CUTOVER_BACKUP | PASS | `deopt-production-backup.service` success before cutover; R2 object `20261104T064250Z-40b1fa1d02e5f7b57250db27ce6d5928.dump.gpg`, 48,514 bytes; remote SHA-256 `1d64488919b1b0048014ff2e55c8cef64eb53f59d653099398ea01b2935d3a93`. Existing objects retained. |
| POSTGRES_VOLUME | PASS | Docker mounted `dxco90jwpdxyennmquv2elyj_postgres_data` after container recreation; no volume deletion. Socket volume is separate. |
| POSTGRES_MIGRATION_0072 | PASS | 71 migrations before run, 72 after; 96 public tables after. Migration used the dedicated `deopt_migrator` role. The V3 real-data restore test established legacy data equivalence for this migration; a fresh byte-level comparison of live rows to the new encrypted snapshot was not performed. |
| RUNTIME_ROLE | PASS | `deopt_runtime` has no superuser, createdb, createrole, replication or bypassrls attribute; owns zero public tables; schema/database CREATE and TEMP are denied. A runtime-secret login over the socket returned 72 migrations. The backend mounts only `runtime_password`. |
| SCRAM_AUTHENTICATION | PASS | Effective `pg_hba_file_rules`: `local scram-sha-256`, IPv4/IPv6 host reject; reviewed HBA SHA-256 `f6a111eabd4e1fb48efe55bed4fbf3e120e040e3e385ba53b8c63e4084896692`. Runtime and backup roles authenticated successfully. |
| SOCKET_ISOLATION | PASS | PostgreSQL Docker network mode `none`, no published port, socket permissions `postgres:postgres:770`; backend on the Compose `web` network and able to reach the database. Coolify proxy has no PostgreSQL socket mount. The proxy remains attached to a stale `db` network, but the PostgreSQL container is absent from all networks. |
| BACKEND_IMAGE | PASS | Running image `ghcr.io/coriom/deopt-v2-backend@sha256:bdccd619f8761e82d68d0eaff50282370eb805c64dcd02a78095e157b31a71fc`, source commit `ba57b16907c209afe2ab5b0b560b985bd33fe5ca`. |
| BACKEND_HEALTH | PASS | Docker healthy; private `/health=200`, `/ready=200`, anonymous canonical fee history `401`. Chain 84532, read-only and all checked economic flags disabled. Backend user `1001:1001`, read-only root filesystem, all capabilities dropped. |
| WALLET_AUTHENTICATION | PASS | Hosted synthetic-wallet smoke: two independent signed challenges and sessions, own-wallet fee history 200, cross-wallet history 403 in both directions, nonce replay 401, logout 204, revoked session 401. No persistent test private keys or tokens. |
| POST_CUTOVER_BACKUP | PASS | Backup service `Result=success`, remote encrypted object `20261104T070612Z-2972a9e5351fe8d01548a749988b0100.dump.gpg`, 49,400 bytes, remote SHA-256 `ee17317f3ee9c5694aac6e1a389eb5bece606a55bfb2755a9de1d9bc5482585f`. First R2 copy attempt returned 501; the bounded retry succeeded. This new ciphertext was not independently restored. Daily timer remains active. |
| COOLIFY_HEALTH | PASS | Resource shows Running; Coolify and proxy containers healthy; administrative tunnel remains private on `127.0.0.1:8000`. External IP HTTPS certificate verification returned 0 with HTTP 503 at the unpublished root. Port 8080 is not published. |
| ROLLBACK_STATUS | PASS (ready) | Schema-72-compatible image `sha256:213d41af2bda1c0f341d43c32a8827fae558d32ad5a272cb93cbfce34fb46550` is present on Netcup, linux/amd64, same source commit. It was tested on isolated restored data in V3; no production rollback was needed or performed. Never use the old schema-71 image after migration. |

## Execution note

The first migration attempt failed before schema changes because the migrator container ran as `1001:999` and could not read a secret owned by group `1001`. The database still had 71 migrations. The migration script was corrected to `--user 1001:1001 --group-add 999`; a disposable container verified access to both secret and Unix socket before retry. Corrected script SHA-256: `93e5c238c657bc94e78128659b682f6bf677fd93805dcefefbf05986dad8ed60`. The retry completed migration 0072 and grants.

At the final idle sample, backend used 13.76 MiB and PostgreSQL 30.86 MiB; 6.4 GiB host RAM was available, with 145 GiB disk free. These are idle values, not a load-capacity result. No Vercel change, proxy restart, blockchain transaction, trading activation or public API publication occurred.

**FINAL_VERDICT: PASS for private read-only production cutover.** Public Vercel connectivity remains a separate release gate.
