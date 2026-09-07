# DEOPT_SECOND_CHAIN_RUNTIME_ARCHITECTURE_V1 — audit + verdict

## Purpose

Answer the milestone Part B question:

> What would prevent simultaneously instantiating `ChainRuntime(Base)`
> and `ChainRuntime(Arbitrum)` without actually enabling Arbitrum?

by walking every plumbing slot the milestone enumerates and reporting
one of:

- **READY** — the slot is already per-instance / per-chain-safe;
  instantiating two runtimes side by side is a legitimate operation.
- **CODE_WIRING_ONLY** — the slot has no chain-affine singleton, but
  a specific call site currently hard-codes `chain_id` from
  `ChainRuntimeHandle::v1_default()`. Multi-chain callers would
  instead source `chain_id` from their own runtime handle. No
  refactor needed; just per-chain construction.
- **NEEDS_CONFIG_FANOUT** — the slot currently reads a single global
  env var. Multi-chain would require reading a per-chain env var set
  (e.g. `BASE_RPC_URL` + `ARBITRUM_RPC_URL`), then handing the right
  value into each runtime's plumbing.
- **BLOCKED** — a genuine architectural blocker that must be resolved
  before a second runtime can be instantiated.

## Per-slot audit

| Slot                    | Status              | Notes |
|-------------------------|---------------------|-------|
| **RPC**                 | NEEDS_CONFIG_FANOUT | `IndexerConfig::rpc_url`, `ExecutionConfig::rpc_url`, `HybridV2Config::rpc_url` are all `Option<String>` per-config. Making them per-chain requires reading e.g. `BASE_RPC_URL` + `ARBITRUM_RPC_URL` at `AppConfig::from_env` and threading the right value into each runtime's `PerChainPlumbing.rpc_url`. No shared state today — each `HttpJsonRpcProvider` instance owns its own reqwest client. |
| **Deployment registry** | NEEDS_CONFIG_FANOUT | `AppConfig::trading_views` holds contract addresses per surface. Today a single set is loaded. Would need to be indexed by `chain_id`: `HashMap<u64, TradingViewsConfig>` or similar. No cross-chain state. |
| **Indexer cursor**      | READY               | Migration 0062 flipped the primary key to `(chain_id, name)`. `PgRepository::get_indexer_cursor(chain_id, name)` takes a chain id. Two indexers on two chains can share the same cursor name (`"perp_matching_engine"`) without collision. |
| **Block / finality tracking** | READY         | Finality is a stateless computation: latest observed block − `finality_confirmations` from `PerChainPlumbing`. No shared latest-block cache today; each RPC provider computes it independently. Two runtimes trivially maintain independent `last_indexed_block` cursors. |
| **Reorg state**         | READY               | Hybrid V2 reorg tables (migration 0047) are keyed by `deployment_id`, which resolves via `hybrid_v2_deployments.chain_id`. A second-chain deployment would live in a different `deployment_id` row and have its own reorg state. |
| **Oracle reader**       | NEEDS_CONFIG_FANOUT | Currently the perps impact-mid keeper reads oracle addresses from env. Multi-chain would need per-chain oracle address env vars (e.g. `BASE_CHAINLINK_ETH_USD`, `ARBITRUM_CHAINLINK_ETH_USD`). No shared cache today; each keeper instance owns its own price cache keyed by `(market_id, source)`. |
| **Executor context**    | NEEDS_CONFIG_FANOUT | `ExecutionConfig` carries `executor_chain_id`, `executor_private_key`, `executor_from_address`. Multi-chain requires per-chain executor configs (`BASE_EXECUTOR_*`, `ARBITRUM_EXECUTOR_*`). The signer trait `RemoteSigner` is per-instance; there is no shared nonce oracle. |
| **Worker ownership**    | CODE_WIRING_ONLY    | `IndexerRunner`, `HybridV2Runner`, `PerpsFundingWorker`, `PerpsLiquidationWorker`, `PerpsImpactMidKeeper` are all constructed inside `main.rs` and spawned via `tokio::spawn`. Each takes its own `IndexerConfig` / `HybridV2Config` / … Multi-chain: construct one instance per chain, spawn per-chain tasks. No shared task handle, no shared queue. |
| **Health state**        | READY               | Health probes report the caller-passed `chain_id` (see `crate::admin::health_status`). No global "current chain" singleton exists. |

## Confirmed absences (nothing to fix)

- No `lazy_static!` chain id anywhere.
- No `OnceLock<ChainId>` / `OnceCell<ChainRuntime>`.
- No `thread_local!` chain state.
- No process-wide nonce ledger keyed by anything other than
  `(trader, nonce_hex)` (already trader-scoped, chain-safe via
  EIP-712 domain).
- No shared indexer queue.
- No global `EIP712Domain`; every signing site derives the domain
  from its owning config, which carries `chain_id`.

## Blockers

**None**. Every plumbing slot is either already per-instance or can
be lifted to per-chain by reading a chain-scoped env var set. A
future binary that instantiates
`ChainRuntime::build(BASE_SEPOLIA_CHAIN_ID, base_plumbing)?` and
`ChainRuntime::build(ARBITRUM_CHAIN_ID, arbitrum_plumbing)?` would
type-check today (once Arbitrum is added to the `ChainConfig`
registry with `enabled = true`), spawn independent workers, and
maintain independent cursors, positions and executor state.

## Non-blockers explicitly out of scope

The following are absent by design (see
`DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1.md`) and MUST remain
absent even after multi-chain activation:

- No bridge / cross-chain messaging.
- No cross-chain nonce coordinator.
- No cross-chain shared margin ledger.

## Verdict

`DEOPT_SECOND_CHAIN_RUNTIME_ARCHITECTURE_READY` (with the caveats
that (a) RPC / deployment / oracle / executor slots require per-chain
env-var fanout at `AppConfig::from_env` and (b) worker spawn sites in
`main.rs` need to loop over `enabled_chains()` instead of taking the
single-chain path — both are wiring changes, not schema or protocol
changes).
