# DeOpt replacement PERP fee accounting

This release records a D2 fee only after an indexed D6 `TradeExecuted` is reconciled with a successful, canonical receipt and the corresponding D2 `FeeChargedV2`. It does not charge, transfer, settle, or broadcast value. The pre-existing local matcher fill and legacy `fee_events` remain separate projections.

## Accounting boundary

- `src/fees/d2_onchain.rs` verifies all five reviewed D2 execution receipts, a fresh pinned Base Sepolia D2 readback, recipient, rebates, and all five 0/200 ppm profiles. A queued operation does not satisfy this proof.
- `src/fees/v2_perp_settlement.rs` resolves maker and taker from the indexed D6 trade, verifies the D1 market's settlement token and mUSDC decimals at the trade block, reads each D2 tier at that block, requires the trade to follow all five D2 execution blocks, and checks the receipt's transaction, block, event identity, payer, recipient, asset, fee rate, basis, and charged amount. D4's two floor conversions and D2's ceiling fee rounding are reproduced using checked integer arithmetic.
- `src/db/repository.rs` inserts the indexed trade and canonical fee row in one PostgreSQL transaction. The `(chain_id, tx_hash, trade_log_index)` trade key and `(chain_id, tx_hash, fee_log_index)` charge key prevent duplicate accrual. A retry returns a no-op only when all stored economic fields agree; a divergent replay or reorg requires explicit reconciliation.
- Confirmation of a V2 trade fails closed if any D2 proof, RPC read, canonical receipt, or database write fails. A canonical fee record is an observation of an on-chain charge, not an instruction to charge again.

## Operational limits

`D2_PERP_TIER_S2_TX_HASHES` is an optional five-hash configuration input for future canonical S2 receipts. It must be absent until each execution has a separate successful public postflight. Even when supplied, the backend independently verifies the receipts and D2 readback on every V2 confirmation. This input cannot enable trading. Public broadcasting and PERP trading stay disabled.

The D6 trade event identifies wallet parties, not backend subaccount IDs. The canonical D2 ledger therefore attributes charges to the on-chain payer wallet; it cannot prove a subaccount split. Local subaccount fee estimates must not be treated as settled fees. If a receipt contains multiple D2 charges or unrelated D6 trades, reconciliation stops for manual review rather than guessing a pairing.

An indexed trade may already have been committed by the indexer before fee reconciliation. Confirmation is withheld until the canonical fee row is written. A crash in that interval leaves an unconfirmed, recoverable indexed projection; retry is idempotent. A reorg after a canonical fee row was committed is detected on replay and requires explicit operator reconciliation. The backend does not silently reverse economic history.

The RPC must support historical `eth_call` by block hash for account-tier attribution. An unavailable historical state or an execution in the same block as a trade fails closed and requires manual reconciliation.

## Remaining release gates

Before any future trading activation, independently complete all six D2/D3 S2 public postflights, C02 and market-state governance, migration and accounting review, D4/D6 maintenance release, required contract permissions, actual subaccount accounting design, backend operational controls, and separate broadcaster authorization. No test in this milestone grants those permissions.

The existing local Docker profile uses a persistent PostgreSQL volume, non-root backend user, read-only backend filesystem, `/health` and `/ready`, Base Sepolia chain ID, and disabled execution flags. A future host still needs private network and admin access controls, durable database backups with restore drills, RPC failover, metrics, log retention, and an operator rollback plan. No remote backend was deployed.

## Executed local checks

- The focused fee suite passed 118 tests, including the disposable PostgreSQL transaction, duplicate delivery, concurrent retry, reconnect, divergence, and rollback checks.
- The final source also passed the missing-block-hash regression and a fresh dedicated PostgreSQL run after the receipt error path was hardened.
- The full library suite passed 1,871 tests with local loopback access. The first sandbox-only run had two existing Options mock-server tests fail at socket bind with `EPERM`; both passed on the authorized loopback rerun.
- A custom-format `pg_dump` of the disposable database restored into a second disposable database. Both had two indexed trades, two canonical fee records, zero legacy fee events, and the same aggregate digest (`cd790e5a28df860c82bdd9b48a06b83a`). No real user data was involved.
- `cargo build` passed. Strict library Clippy still fails on 196 repository diagnostics; the new fee path has no reported Clippy diagnostics. Global `cargo fmt --check` remains blocked by unrelated pre-existing formatting drift; the new fee files pass `rustfmt --check`.
- The final source revision built as local image `deopt-alpha-backend:d2-atomic-v1` (`sha256:28a064c7f3dee6c331d4d16ad58a9f0ff634480c5a63ce8dd8c881c80b546ffa`). Its non-root, read-only container started against fresh synthetic PostgreSQL. `/health` and `/ready` returned 200, market reads returned 503, trading and workers remained disabled, and the database retained exactly two indexed trades, two canonical fees and zero legacy fee events. The preceding image also restarted successfully against the restored database before the final receipt-only hardening.
- Connected frontend fee display, an isolated V2 on-chain execution fixture, and live canonical D2 S2 readbacks were not performed in this milestone. Local unit and PostgreSQL results cannot stand in for those gates.
