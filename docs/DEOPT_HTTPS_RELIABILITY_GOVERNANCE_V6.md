# DEOPT_HTTPS_RELIABILITY_GOVERNANCE_V6

Read-only snapshot: 2026-11-04 08:50 UTC. No production configuration or chain state was changed. The monitoring workflow in this commit is **not active until a separately approved push and GitHub Actions run**.

## HTTPS

| Gate | Verdict | Evidence |
| --- | --- | --- |
| CURRENT_TLS_CERTIFICATE | PASS | Strict external TLS verification to `94.16.113.122` without DNS SNI: IP SAN matches; SHA-256 fingerprint `544a62862f97b143e942bd688ef4a922a43242c26c357ff1f51de20fa2dcbf02`; expires `2026-11-10T06:20:04Z`. |
| ACME_CONFIGURATION | PASS for loaded settings | Read-only `docker inspect coolify-proxy`: Traefik 3.7 uses `deoptipproduction` and `deoptipstaging`, HTTP-01 entrypoint `http`, `profile=shortlived`, `disablecommonname=true`, `certificatesduration=160`, separate storage files. Production `/traefik/acme-ip-production.json` is a bind mount, root-owned mode 0600, nonempty (15825 bytes). No file contents were read. |
| REAL_RENEWAL | NOT_YET_OBSERVED | The externally served fingerprint still matches the previous observed certificate. A filtered count found one ACME/certificate-related proxy log line in the preceding 24 hours; that count does not prove a renewal attempt or success. No public ACME issuance or proxy restart was triggered. |
| VERCEL_NETCUP_HEALTH | PASS at snapshot | Strict TLS and JSON health checks returned HTTP 200 for `https://94.16.113.122/health` and `https://deoptv2-frontend.vercel.app/api/deopt/public/health`. |
| EXTERNAL_MONITORING | BLOCKED activation | `deploy/vps/monitor_public_https_v6.py` validates IP SAN, chain of trust, certificate expiry with a 48-hour warning threshold, Netcup health, and Vercel gateway. `.github/workflows/deopt-public-https-monitor.yml` schedules hourly at minute 17 and supports a synthetic failure drill. It has not been pushed by policy. Scheduled GitHub Actions may be delayed and can be disabled after repository inactivity; this is best-effort monitoring, not an uptime guarantee. |
| ALERT_DELIVERY | BLOCKED | Local synthetic failure returns nonzero and a GitHub Actions error annotation. GitHub notification delivery cannot be claimed until the workflow is published, manually drilled, and receipt confirmed by an operator. The 48-hour threshold begins `2026-11-08T06:20:04Z` for the current certificate. |

Commands used: `docker inspect coolify-proxy --format '{{range .Config.Cmd}}{{println .}}{{end}}'` with ACME-only filtering; `docker exec coolify-proxy stat -c ... /traefik/acme-ip-production.json`; external Python SSL probe; local unit tests and a live monitor run. No secret-bearing ACME state or logs were exported.

## Base Sepolia governance

Pinned canonical block **54,299,222**, hash `0x74194974bc702af491532613fc15cd465f05a11ef09b91fb792b5ad406dade91`, time `2026-11-04T08:41:53Z`, chain ID 84532. The same block hash was read twice. Public RPC rate limits caused a few supplementary calls to fail; the results below include only successful reads. This is a snapshot, so every execution preflight must be refreshed.

| Gate | Verdict | Evidence |
| --- | --- | --- |
| SAFE | PASS at block | OPS Safe nonce 43, threshold 2 of 3; owners `0xb9f8de807ff98d5730035a8bfed4bda33e886d06`, `0x0e7dcb5b9fd969e4fdc6f3a7b7993819ddc5a35d`, `0xa774c46c41064524df895bd7be9f294409798dfc`. |
| C02_CURRENT_STATUS | READY, not executed | Timelock operation `0x9e10ee1c04280e36ba1d7144b5abd2d1a48b08fac54087bd5e7090d91030913e`: exact queued operation true, `isOperationReady` true. Timelock minimum delay 86400 s and grace period 1209600 s. |
| C02_EXPIRATION | `2026-11-08T12:00:00Z` inclusive | ETA `2026-10-25T12:00:00Z` from canonical queue evidence; Timelock source rejects only timestamps greater than ETA + 14 days. After expiry it cannot execute; a new reviewed scheduling operation is required. |
| MKT1_STATUS | READY, not executed | Operation `0x9d2bd83ac8d34252fa2418e5b403a716e0131121fd14dad25a112220f7eef268` queued and ready. ETA `2026-11-01T12:00:00Z`; expiry `2026-11-15T12:00:00Z` inclusive. Its execution still requires C02 canonical execution postflight and separate Safe approval. |
| MARKET_AND_MAINTENANCE | PASS at block | D1 total markets 0, market 1 absent. D4 trading/liquidation/funding/collateral operations paused; D6 paused. D4 points to D5. D2 five PERP tiers read maker 0/taker 200 ppm; recipient Timelock. D3 mUSDC enabled at 5 bps. D5 maxOracleDelay 600 s and D1 creation/config pauses false in subsequent live reads. |
| ORACLE | PASS for one feed at block | OracleRouter reports active mWETH/mUSDC feed, `getPriceSafe` ok, price `232406211681` at 1e8 scale, updatedAt 1793781418 (~295 s old at block). This does not validate every market parameter or future freshness. |
| REMOTE_EXECUTOR_ISOLATION | BLOCKED | Current Netcup Coolify terminal listed only Coolify and the read-only DeOpt backend/PostgreSQL containers. Vercel production project `prj_YADz33AWNuLnbnXYvQZOINFIhTTG` reports READY deployment `dpl_5ryUVNmVQR8tmL5pGEDQJc5g1fwY`; checked frontend API source contains no cron or broadcaster configuration. These observations do not establish a complete production Functions/Cron inventory or the status of historical Railway/other keeper, relayer, broadcaster, or Rust instances. Those remain UNKNOWN. |
| PERPS_GOVERNANCE_READINESS | BLOCKED | `PerpMarketRegistry.createMarket` writes `isActive=true` immediately. MKT1 `setMarketStatus(1,false,false)` is a separate Safe/Timelock action; its delay is ready, but the active interval is unbounded if confirmations, RPC, or execution fail. Existing D4/D6 pauses are useful containment, not proof that every remote economic path is isolated. C02 must not be executed merely to beat expiry. |

Shortest conditional path: independent remote-executor isolation evidence and source-backed C02 containment review; separate C02 S2 approval and manual Safe 2-of-3 execution; canonical C02 postflight; fresh MKT1 state/nonce review; separate MKT1 S2 approval and manual execution; canonical market-1 INACTIVE postflight. If isolation is not demonstrated before C02 expiry, let it expire and review a fresh Timelock schedule. Reuse of queued MKT1 after a replacement C02 requires a fresh compatibility and expiry review.

## Product gates

Perps require market creation/deactivation, then reviewed market-2/oracle/risk wiring, executor/guardian and shared ACL preparation, migration equality/seal, restricted activation, backend indexation and authenticated frontend trading flow, isolated end-to-end economic tests, and separate governance approvals. Current backend trading and broadcasters remain disabled. Options have independent contract, market, oracle, liquidity, settlement, backend and frontend acceptance gates; Perps evidence does not establish Options readiness. Neither is authorized for public trading.

## Validation and next action

Focused local tests: 3 unit tests PASS; workflow static policy check PASS; live strict TLS monitor PASS once run with network permission. No GitHub alert delivery test, actual ACME renewal, public-chain transaction, or production change occurred. No push is authorized in this milestone.

**Next decision before Nov 8:** obtain a timestamped, provider-backed inventory or explicit operator attestation covering every remote Rust backend, Railway service, Vercel Function/Cron, relayer, keeper and broadcaster able to submit Base Sepolia transactions. If any status remains UNKNOWN, C02 stays blocked and should be requeued after expiry rather than executed into an active market.
