# `BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1`

Date landed: 2026-08-04
Milestone id: `BACKEND-HYBRID-V2-LIVE-CHAIN-SOURCE-AND-WORKER-ACTIVATION-V1`
Predecessors:
- `BACKEND-HYBRID-V2-POSTGRES-PROJECTION-CORE-V1`
- `BACKEND-HYBRID-V2-POSTGRES-READ-STORE-V1` (2A + 2B)
- `BACKEND-PG-MIGRATION-CHAIN-INTEGRITY-V1`
- `BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` (stage 3A)

Status: `IMPLEMENTED_AND_VALIDATED_EXPERIMENTAL`
Safety posture: `EXPERIMENTAL — NOT SECURITY APPROVED`

## Purpose

Replace the log-only, deferred worker branch introduced in
`BACKEND-HYBRID-V2-PERSISTED-RUNTIME-CORE-V1` with a real,
production, supervised persisted Hybrid V2 indexer worker driven by
a strictly read-only EVM JSON-RPC `ChainSource`.

This milestone is the narrow prerequisite between the persisted
runtime core (stage 3A) and the operational reorg recovery stage.
It does **not** implement operational reorg replay, orphan
invalidation, rebuild, or reconciliation. Those remain deferred to
`BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` and
`BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1`.

## Frozen posture

- `CHAIN_STATE_IS_CANONICAL_POSTGRES_IS_A_REBUILDABLE_NON_CANONICAL_PROJECTION`
- `PRODUCTION_HYBRID_V2_HTTP_READS_USE_POSTGRES_ONLY`
- `NO_RUNTIME_STATE_PUBLICATION_BEFORE_POSTGRES_COMMIT`
- **New**: `HYBRID_V2_CHAIN_SOURCE_IS_STRICTLY_READ_ONLY` — the
  `ChainSource` trait itself exposes no write method, and
  `RpcHybridV2ChainSource` defense-in-depth rejects any prohibited
  method (`eth_sendTransaction`, `eth_sendRawTransaction`, `eth_sign`,
  `eth_signTransaction`, `eth_accounts`, `personal_sign`,
  `personal_unlockAccount`, `wallet_*`) at the client boundary.
- **New**: `HYBRID_V2_ENABLED_AND_CONFIGURED_STARTS_ONE_SUPERVISED_PERSISTED_WORKER` —
  when `HYBRID_V2_ENABLED=true` AND `PERSISTENCE_ENABLED=true` AND a
  valid configuration + manifest is loaded, exactly one supervised
  worker is spawned. Startup failure (bad chain id, Base mainnet,
  manifest mismatch, RPC unreachable, `BootstrapResult::Diverged`)
  fails-closed before `axum::serve` binds.
- No frontend / Solidity / signer / broadcast / chain-write.
- Base mainnet (chain_id 8453) refused unconditionally by
  `HybridV2Config::validate`, by
  `IndexerRuntime::validate_manifest_binding`, and by
  `RpcHybridV2ChainSource::validate_chain_identity`.

## `ChainSource` trait extension (narrow, authorised)

Old (sync, no error type):

```rust
pub trait ChainSource: Send + Sync {
    fn chain_id(&self) -> u64;
    fn head_block_number(&self) -> u64;
    fn block_at(&self, number: u64) -> Option<RawBlock>;
    fn block_by_hash(&self, hash: &str) -> Option<RawBlock>;
    fn finalized_block_number(&self) -> u64 { 0 }
}
```

New (async, `Result` returns, typed error):

```rust
#[async_trait]
pub trait ChainSource: Send + Sync {
    async fn chain_id(&self) -> Result<u64, ChainSourceError>;
    async fn head_block_number(&self) -> Result<u64, ChainSourceError>;
    async fn finalized_block_number(&self) -> Result<u64, ChainSourceError> { Ok(0) }
    async fn block_at(&self, number: u64) -> Result<Option<RawBlock>, ChainSourceError>;
    async fn block_by_hash(&self, hash: &str) -> Result<Option<RawBlock>, ChainSourceError>;
}

pub enum ChainSourceError {
    Transport(String), Timeout, RateLimited, ServerError { status: u16 },
    Malformed(String), RpcError { code: i64, message: String },
    Unsupported(String), Cancelled,
}
```

`InMemoryChainSource`, `IndexerRuntime::tick` / `tick_and_persist`,
`ReorgPlanner::plan`, and every test that touched the trait were
updated. `RuntimeError` gained a `ChainSource { detail: String }`
variant used when a transient source failure aborts a tick without
advancing the cursor.

## `RpcHybridV2ChainSource`

New in `src/hybrid_v2/rpc_chain_source.rs`.

- **Underlying HTTP client**: `reqwest::Client` (already vendored)
  built with `.timeout(rpc_timeout_ms)`; no new crate added.
- **Endpoint redaction**: `Debug` and `Display` never emit the raw
  URL; `redacted_endpoint()` returns only the host segment (or
  `<opaque>` on parse failure). `HybridV2Config::Debug` similarly
  redacts.
- **Allowed methods** (defense-in-depth allowlist, matched at
  `call(...)`): `eth_chainId`, `eth_blockNumber`,
  `eth_getBlockByNumber`, `eth_getBlockByHash`, `eth_getLogs`.
- **Prohibited methods** (rejected before the request leaves the
  process): `eth_sendTransaction`, `eth_sendRawTransaction`,
  `eth_sign`, `eth_signTransaction`, `eth_accounts`, `personal_sign`,
  `personal_unlockAccount`, `wallet_*`.
- **Chain identity**: `validate_chain_identity()` queries
  `eth_chainId`, compares against `HYBRID_V2_CHAIN_ID`, and returns
  `ChainSourceError::Unsupported` on mismatch or Base mainnet.
- **Finality**: probe order `finalized` → `safe` → confirmation
  depth fallback (head - depth). Whichever tag succeeds first is
  cached (`OnceCell`) for the process lifetime.
- **Log acquisition**: `eth_getBlockByNumber(hex, false)` for the
  header, then `eth_getLogs { fromBlock: hex, toBlock: hex, address:
  [emitters..] }` for the same block. Post-fetch invariants:
  - every returned log's `blockHash` matches the header;
  - `logs.len() <= rpc_max_logs_per_range` (bounded);
  - deterministic ordering by `(log_index ASC)`;
  - deduplication by `(blockHash, txHash, logIndex)`;
  - topics/data hex parsing errors → `ChainSourceError::Malformed`.
- **Retry policy** (retryable set): transport error, `Timeout`,
  HTTP 429 (`RateLimited`), HTTP 5xx (`ServerError`). Non-retryable:
  any `RpcError` (deterministic provider error), `Malformed`,
  `Unsupported`, chain-id mismatch. Bounded exponential backoff
  (`rpc_retry_backoff * 2^attempt`, capped) up to `rpc_max_retries`.
  Sleeps are plain `tokio::time::sleep`; higher-level shutdown
  cancels the spawned task.

## `HybridV2Config` extension

New fields (all required at `validate()` when `enabled=true`):

| Field | Env var | Bounds |
|---|---|---|
| `rpc_url: Option<String>` | `HYBRID_V2_RPC_URL` | non-empty, `http(s)://` |
| `rpc_timeout_ms: u64` | `HYBRID_V2_RPC_TIMEOUT_MS` | [500, 60_000], default 10_000 |
| `rpc_max_retries: u32` | `HYBRID_V2_RPC_MAX_RETRIES` | [0, 10], default 3 |
| `rpc_retry_backoff_ms: u64` | `HYBRID_V2_RPC_RETRY_BACKOFF_MS` | [50, 10_000], default 250 |
| `rpc_max_logs_per_range: u32` | `HYBRID_V2_RPC_MAX_LOGS_PER_RANGE` | [1, 20_000], default 2_000 |
| `manifest_path: Option<String>` | `HYBRID_V2_MANIFEST_PATH` | non-empty file path |

`Debug` impl for `HybridV2Config` is manual and redacts `rpc_url`
to just the host or `<redacted>`. Seven new unit tests cover the
new validation rules and the redaction contract.

## `main.rs` — real supervised worker startup

The former deferred branch at lines 176-195 is replaced with a
strict pipeline that fails-closed at every step:

1. `HYBRID_V2_ENABLED=false` (default) → silent skip.
2. `HYBRID_V2_ENABLED=true` but `PERSISTENCE_ENABLED=false` → WARN
   and skip; canonical routes remain fail-closed at the API
   boundary.
3. `HYBRID_V2_ENABLED=true` AND `PERSISTENCE_ENABLED=true`:
   a. Load `ManifestParams` from `HYBRID_V2_MANIFEST_PATH`
      (`serde_json::from_slice`).
   b. Assert `manifest.chain_id == HYBRID_V2_CHAIN_ID`.
   c. Collect canonical emitter addresses (all module addresses +
      manifest address, lowercased) from the manifest.
   d. Build `RpcHybridV2ChainSource::new(config, emitters)`.
   e. `source.validate_chain_identity().await` — mismatch or Base
      mainnet returns `Err`, `main` propagates, process exits.
   f. Construct `PostgresHybridV2ProjectionStore` sharing the
      application `PgPool`.
   g. Build `IndexerRuntime::new(deployment_id, manifest)
      .with_persistence(store, deployment_id)
      .with_persistence_cursor_name(cursor_name)`.
   h. `runtime.bootstrap_from_persistence().await` — BOOTSTRAP-2
      journal replay; `BootstrapResult::Diverged` or
      `::ChainForbidden` are startup errors.
   i. `spawn_hybrid_v2_indexer_worker(runtime, source, store,
      worker_config, Some(shutdown_rx))` — returns a `JoinHandle`;
      the `watch::Sender` is kept alive on the stack.
   j. `info!(...)` logs the configured state with the RPC endpoint
      **redacted**.

All steps a-i return before `axum::serve` binds; a mis-configured
indexer never coexists with a live HTTP surface.

## Test surface

| File | Tests | Notes |
|---|---|---|
| `tests/hybrid_v2_rpc_chain_source_mock_integration.rs` | 19 | Mock JSON-RPC server; chain-id ok/mismatch/mainnet, head, block-null, block-with-logs, multi-emitter filter, dupe collapse, malformed block hash, parent mismatch, finality tag support/unsupport/fallback, HTTP 429/500 retry, non-recovery final error, non-retryable RPC error, cancellation, prohibited-method allowlist audit. |
| `tests/hybrid_v2_rpc_chain_source_properties.rs` | 9 | Bounded properties: log ordering, dedupe, retryable-failures-preserve-result, deterministic-error-not-retried, disabled config never RPCs, wrong-chain never polls, no prohibited method generated, empty-block roundtrip, block-at/block-by-hash agree. |
| `tests/hybrid_v2_live_worker_pg_integration.rs` | 8 (PG-gated) | Real Postgres + mock RPC + real main router: disabled/enabled/wrong-chain/mainnet, empty-block cursor advance, failed-RPC cursor unchanged, graceful shutdown, restart resume, parent-mismatch cursor unchanged. |
| `tests/hybrid_v2_mock_rpc_helpers.rs` | (harness) | Deterministic axum-based mock RPC server; ships with prohibited-method rejection. |

Full workspace test count: **1200 lib + 700+ integration/property**,
green against real PostgreSQL 16.14. Three pre-existing legacy PG
isolation failures remain documented and unchanged.

## Files landed

- `src/hybrid_v2/chain_source.rs` — async trait + `ChainSourceError`.
- `src/hybrid_v2/rpc_chain_source.rs` — **new**, ~780 LOC, the
  production RPC ChainSource + allowlist enforcement.
- `src/hybrid_v2/config.rs` — extended with 6 new fields, `Debug`
  redaction, 7 new unit tests.
- `src/hybrid_v2/reorg.rs` — async `plan()`.
- `src/hybrid_v2/runtime.rs` — async `tick` / `tick_and_persist`;
  new `RuntimeError::ChainSource` variant.
- `src/hybrid_v2/mod.rs` — module registration + re-exports.
- `src/main.rs` — real worker spawn + `HYBRID_V2_MANIFEST_PATH`
  loader + `collect_manifest_emitters` helper.
- `src/db/repository.rs` — `pool()` accessor.
- `tests/hybrid_v2_mock_rpc_helpers.rs` — mock RPC harness.
- `tests/hybrid_v2_rpc_chain_source_mock_integration.rs` — 19 tests.
- `tests/hybrid_v2_rpc_chain_source_properties.rs` — 9 tests.
- `tests/hybrid_v2_live_worker_pg_integration.rs` — 8 tests.
- Test callsite updates: `hybrid_v2_runtime_tests.rs`,
  `hybrid_v2_property_tests.rs`, `hybrid_v2_read_api_tests.rs`,
  `hybrid_v2_read_api_main_router_tests.rs`,
  `hybrid_v2_read_api_property_tests.rs`.
- `docs/BACKEND_HYBRID_V2_LIVE_CHAIN_SOURCE_AND_WORKER_ACTIVATION_V1.md` — this file.

Cargo.toml + Cargo.lock: unchanged. No new dependencies.

## Security posture

- No signer, no private key, no wallet code introduced in the
  `hybrid_v2` module. Verified by static grep.
- No RPC URL, credential, or path emitted by `Debug`, `Display`, or
  any log statement — verified by `debug_impl_never_leaks_rpc_url_path_or_key`
  test and by grep sweep.
- All broadcast/signing RPC methods rejected at the client boundary
  before a request leaves the process. Confirmed by
  `prohibited_method_not_requested_ever` integration test and
  `prop_no_prohibited_method_generated` property.
- Bounded resources: retries capped, backoff capped, `max_logs_per_range`
  enforced, timeout on every RPC call.
- Fail-closed at every startup step; process exits non-zero on
  invalid configuration rather than binding the HTTP surface behind
  a mis-configured indexer.

## Out of scope (deferred stages)

- `BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1` — operational
  reorg detection under `tick_and_persist`, orphan block/log
  invalidation, canonical replay of the replacement chain,
  restart-during-reorg.
- `BACKEND-HYBRID-V2-PROJECTION-PERSISTENCE-CLOSURE-V1` — deployment
  rebuild lock, full rebuild, chain-view reconciliation, final
  closure.
- Signer / execution / broadcast integration.
- Base Sepolia read-only real-network smoke test (may run manually
  by the operator when they set the endpoint env var; not gated in
  CI).

## Exact next stage

`BACKEND-HYBRID-V2-PERSISTED-REORG-RECOVERY-V1`.
