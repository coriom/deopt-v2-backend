# DEOPT_MULTICHAIN_SCHEMA_HARDENING_V1 — audit + Part A verdict

## Purpose

Complete the DB-identity work that migration 0061 started additively.
This document enumerates every table with a chain-derived write path,
classifies its canonical-identity posture, and proves the two
remaining legacy uniqueness constraints (`indexer_cursors.name` and
`indexed_perp_trades.UNIQUE(tx_hash, log_index)`) have been removed
by migration 0062 with matching Rust repository updates.

## Audit table

For every table with a chain-derived write path, the audit column is
one of:

- **DEPLOYMENT_SCOPED** — table already includes `deployment_id` as
  the leading key column. `deployment_id` is a `BIGSERIAL` foreign key
  into `hybrid_v2_deployments`, which is itself uniquely keyed by
  `(chain_id, manifest_hash)` (see migration 0044). This is a
  strictly stronger namespace than `chain_id` alone and requires no
  change.
- **BACKEND_UUID** — table's row identity is a backend-generated UUID
  (or backend-generated TEXT id like a series id). No two chains can
  independently produce the same UUID, so cross-chain collision is
  impossible even without a `chain_id` column.
- **CHAIN_SCOPED** — table now carries `chain_id` as part of its
  primary / unique key (via migration 0025, 0061 + 0062).
- **INTENT_HASH_KEYED** — table's PK is a keccak-256 hash whose
  preimage embeds `chain_id` (via EIP-712 domain or SubKey), so cross-
  chain collision is cryptographically impossible.
- **INTENTIONALLY_DEFERRED** — table has a plausible cross-chain
  collision vector, but the platform will NEVER touch it from more
  than one chain in V1 (Base only) and code-level defenses are already
  in place. Documented for future activation.

| Table                                       | Migration | Audit                    | Notes |
|---------------------------------------------|-----------|--------------------------|-------|
| `indexer_cursors`                           | 0004 + 0061 + 0062 | **CHAIN_SCOPED**            | PRIMARY KEY flipped from `(name)` to `(chain_id, name)` in 0062. |
| `indexed_perp_trades`                       | 0004 + 0061 + 0062 | **CHAIN_SCOPED**            | Legacy `UNIQUE(tx_hash, log_index)` dropped in 0062; retains `UNIQUE(chain_id, tx_hash, log_index)`. |
| `option_execution_events`                   | 0025      | **CHAIN_SCOPED**            | Was already born with `UNIQUE (chain_id, tx_hash, log_index)`. |
| `option_execution_reconciliations`          | 0026      | **BACKEND_UUID**            | PK `id UUID`; joined by `option_execution_transaction_id` (backend-generated). |
| `option_execution_transactions`             | 0021      | **BACKEND_UUID**            | PK `transaction_id TEXT` (backend-generated). |
| `option_execution_intents`                  | 0019 + 0054 | **INTENT_HASH_KEYED**      | PK `intent_id TEXT` = intent hash embedding EIP-712 domain (chain_id). |
| `option_execution_correlations`             | 0055      | **BACKEND_UUID**            | PK `correlation_id UUID`. |
| `option_reservations`                       | 0057      | **BACKEND_UUID**            | PK `reservation_id UUID`. |
| `option_series`                             | 0012      | **INTENTIONALLY_DEFERRED**  | `option_series_id TEXT` is a backend-computed id from series metadata. No chain in the preimage today; second-chain activation would need to include chain_id in the series-id derivation. Documented in `DEOPT_MULTICHAIN_MULTICOLLATERAL_FOUNDATION_V1.md`. V1: single-chain, no collision. |
| `option_orders` / `option_fills`            | 0013 / 0014 | **BACKEND_UUID**          | PK `order_id TEXT` / `fill_id TEXT`, both backend-generated. |
| `option_rfqs`                               | 0010 + 0015 | **BACKEND_UUID**          | PK `option_rfq_id TEXT`. |
| `option_multi_leg_rfqs`                     | 0043      | **BACKEND_UUID**            | PK `option_rfq_id TEXT`. |
| `option_execution_correlations_submission_unknown` | 0056 | **BACKEND_UUID**       | Inherited PK. |
| `perp_positions`                            | 0033 + 0036 | **INTENTIONALLY_DEFERRED** | PK `id UUID` (safe). Partial unique `(lower(account), market_id) WHERE status='open'` would collide across chains if two chains published the same `market_id` string. V1: single-chain. |
| `perp_orders` / `perp_fills`                | 0034      | **BACKEND_UUID**            | Both PKs are UUIDs. |
| `perp_liquidations`                         | 0035      | **BACKEND_UUID**            | PK `id UUID`. |
| `perp_funding_events`                       | 0036      | **BACKEND_UUID**            | PK `id UUID`; joined by `position_id`. |
| `perps_signed_intent_nonces`                | 0059      | **INTENT_HASH_KEYED**       | PK `(trader, nonce_hex)`; nonce_hex is derived by the wallet inside the EIP-712 payload (chain-scoped by domain). |
| `perp_intent_fills`                         | 0060      | **INTENT_HASH_KEYED**       | PK `intent_hash` = EIP-712 intent hash (chain-scoped). |
| `subaccounts`                               | 0038      | **INTENTIONALLY_DEFERRED**  | PK `(owner_address, subaccount_id)`. Two chains with the same wallet + subaccount id would collide *at the DB row level*. V1 has one chain runtime so no writer produces conflicting rows. Second-chain activation will require a schema flip to `(chain_id, owner_address, subaccount_id)` — checklist enumerated in `DEOPT_SECOND_CHAIN_ACTIVATION_CHECKLIST_V1.md`. |
| `fee_ledger` (0018)                         | 0018      | **BACKEND_UUID**            | PK `fee_event_id TEXT`; every unique constraint uses backend-generated column subsets. |
| `execution_intents` (0001)                  | 0001      | **INTENT_HASH_KEYED**       | Backend intent-hash PK. |
| `execution_reconciliations` (0006)          | 0006      | **BACKEND_UUID**            | PK `reconciliation_id TEXT`; `UNIQUE(intent_id, indexed_event_id)` — both fields backend-generated. |
| `execution_transactions` (0007)             | 0007      | **BACKEND_UUID**            | PK `transaction_id TEXT`. |
| `execution_simulations` (0003)              | 0003      | **BACKEND_UUID**            | PK `simulation_id TEXT`. |
| `execution_intent_signatures` (0002)        | 0002      | **INTENT_HASH_KEYED**       | Keyed on `intent_id` (FK to execution_intents). |
| `option_twap_orders` / children             | 0037      | **BACKEND_UUID**            | Both TEXT ids. |
| `rfqs` / `quotes` (legacy)                  | 0010      | **BACKEND_UUID**            | Both TEXT ids. |
| `mm_permissions`                            | 0017      | **BACKEND_UUID**            | PK `mm_account TEXT`. |
| `write_auth_challenges`                     | 0029      | **INTENT_HASH_KEYED**       | PK `nonce_bytes BYTEA`; nonces are random challenges, no chain collision. |
| Any `hybrid_v2_*` table (0044–0053)         | 0044–0053 | **DEPLOYMENT_SCOPED**       | Every PK/unique includes `deployment_id`. Namespace is stronger than chain_id alone. |

## Migration 0062 — what changed

- `indexer_cursors`: drop `PRIMARY KEY (name)`, add `PRIMARY KEY
  (chain_id, name)`. Drops the redundant composite unique index from
  migration 0061.
- `indexed_perp_trades`: drop the legacy `UNIQUE (tx_hash, log_index)`
  constraint. Retains `UNIQUE (chain_id, tx_hash, log_index)` from
  migration 0061.

Both changes are safe because every existing row was backfilled to
`chain_id = 84532` by migration 0061.

## Rust repository updates (shipped in the same commit)

- `PgRepository::get_indexer_cursor(name)` →
  `PgRepository::get_indexer_cursor(chain_id, name)`.
- `PgRepository::persist_indexed_perp_trades_and_cursor(cursor_name, trades, last_block)` →
  `PgRepository::persist_indexed_perp_trades_and_cursor(chain_id, cursor_name, trades, last_block)`.
- Internal helpers `upsert_indexer_cursor` and
  `insert_indexed_perp_trade` gained a `chain_id` parameter and their
  `ON CONFLICT` clauses now name the chain-scoped columns.
- Call sites (`indexer::runner::tick`, `api::routes::indexer_status`)
  now source `chain_id` from
  `ChainRuntimeHandle::v1_default().chain_id()`. Multi-runtime
  callers would source the id from their own handle.

## Rollback posture

If migration 0062 needs to be reverted:

1. Recreate `indexer_cursors.PRIMARY KEY (name)` (data preserved,
   still uniqueness-compatible in single-chain mode).
2. Recreate `UNIQUE (tx_hash, log_index)` on `indexed_perp_trades`
   (same argument).
3. Redeploy the previous Rust binary that queries by `name` only /
   `ON CONFLICT (tx_hash, log_index)`.

No data loss.

## Verdict

`DEOPT_MULTICHAIN_DATABASE_IDENTITIES_VALIDATED`
