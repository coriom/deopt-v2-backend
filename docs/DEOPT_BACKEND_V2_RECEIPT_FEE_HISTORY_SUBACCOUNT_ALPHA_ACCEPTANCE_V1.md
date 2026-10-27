# DeOpt V2 receipt, fee history, and subaccount acceptance

## Economic identity

The frozen D6 `TradeExecuted` event identifies buyer and seller wallets, market, size, price, maker side, nonces, and intent ID. It does **not** identify either backend subaccount. `executeTrade` and `executeBatch` use this event. D6's separate `TradeExecutedFromIntents` event carries the signed intent hashes, market, size, price, and timestamp; subaccount IDs are in the signed transaction arguments, not this event. The current indexer decodes `TradeExecuted` only. A receipt containing only `TradeExecutedFromIntents` cannot be passed through the existing fee reconciler as a wallet or subaccount fill.

The backend now exposes `GET /accounts/{wallet}/perps/canonical-fees` from the PostgreSQL canonical D6/D2 ledger. The route validates the wallet address, requires Base Sepolia and persistent storage, rejects query parameters that might imply subaccount filtering, caps the response at 100 rows, and returns per-wallet fee amount and maker/taker role. Every row has `subaccount_id: null` and `attribution: wallet_only_unresolved_subaccount`. The response explicitly says current chain canonicality requires a fresh recheck. This route never joins local projected fills or the legacy fee ledger. It is a separate API contract; the existing frontend fills component does not yet consume it.

Creating, archiving, renaming, or reassociating local subaccounts does not provide proof for assigning an on-chain D6 fee. Neither a default subaccount nor a client order ID is an authoritative on-chain account key. Fee-dependent subaccount settlement remains blocked pending a durable proof from signed D6 intent calldata, matching event hashes, wallet authorization, and the actual contract accounting semantics. The frozen engine currently applies economic state at wallet level; off-chain subaccount bookkeeping must not imply independent on-chain economic balances.

## Receipt processing

The existing D2 reader requires the reviewed five canonical S2 execution receipts, pinned readbacks, the expected 0/200 ppm tiers, zero rebates, and the ProtocolTimelock recipient. It does not activate trading. The fee reconciler checks a D6 receipt against the indexed trade and D2 charge, then persists trade and fee in one PostgreSQL transaction with unique trade and fee-log keys. This milestone additionally binds each charge to the D6 trade immediately following it in a multi-trade receipt. A duplicate charge in that segment fails closed. Replay of the same record is a no-op; conflicting fee or block hash fails and requires operator reconciliation. No automatic reversal of an already-settled ledger row occurs after a reorg.

The wallet-history response reports `receipt_verified_at_ingestion`; it does not assert that an old block remains canonical at read time. A production UI must not display these rows as currently final without fresh canonical-chain reconciliation. The route reports `trading_enabled: false`.

## Frontend interface delta

The current frontend's `/accounts/{wallet}/perps/fills` response represents local matching fills and exposes local subaccount IDs. A future frontend change should consume `/accounts/{wallet}/perps/canonical-fees` as a **separate** wallet-level settled-fee history, display unresolved subaccount attribution, and keep projected fills visually distinct. It must not merge the two lists by client order ID or silently put a canonical fee into Account 1. No frontend repository or Vercel deployment was changed in this milestone.

## Fresh local validation

The library campaign passed 1,874 tests against a disposable PostgreSQL database. The final receipt/fee subset passed 7 tests, including two fills in one transaction, exact retry idempotency, divergent fee and block-hash rejection, and wallet history with two local subaccounts. The final API guard test passed separately. An existing isolated Anvil lifecycle passed one synthetic local transaction with exact vault-balance deltas; it did not run the new D2 fee-history path. The final Docker image built and ran as a non-root user on a read-only filesystem with public execution flags disabled. HTTP health and readiness returned 200; readiness reported perps trading disabled. The synthetic PostgreSQL history returned three distinct wallet rows for both maker and taker, with 0 maker fees, 20,000 native mUSDC units charged to the taker per fill, and null subaccount IDs. A subaccount query returned 400. Both local test containers were stopped after validation.

The touched Rust files pass `rustfmt --check`. The repository-wide `cargo fmt --check` still fails on pre-existing formatting drift. Strict library Clippy still fails with 196 diagnostics, the same count as the preceding accepted milestone; none are in the newly added receipt/history path. The separate frontend browser and actual public D2 execution gates were not validated here.

## Remaining acceptance gates

- Verify all five D2 public S2 executions and current fee readbacks independently. ETA alone is insufficient.
- Extend the indexer and receipt reconciler for `TradeExecutedFromIntents` only with calldata, EIP-712 hash, wallet, subaccount, and event identity proof. Preserve unresolved records when proof is absent or conflicting.
- Prove the complete D6/D2 event to PostgreSQL to API to frontend rendering path with an isolated Anvil trade using the actual approved D2 fee configuration.
- Reconcile historical canonical rows against fresh block hashes after a reorg before claiming current finality in the UI.
- Finish the repository-wide strict Clippy/formatting gates and a connected local frontend browser test.
- Keep public broadcasting and trading disabled until separate market, migration, maintenance, permissions, operational, and operator activation gates pass.

No public chain transaction, backend hosting change, Safe action, or frontend production change is part of this work.

## Future hosting prerequisites

Use a persistent Rust service and PostgreSQL with durable storage, backups, restore drills, readiness checks, log retention, and a bounded indexer replay window. Keep the broadcaster disabled in the image's runtime configuration and independently gate any later activation. Vercel remains the frontend host; no remote Rust service is deployed by this milestone.
