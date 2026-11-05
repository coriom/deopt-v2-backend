# DEOPT_OPTIONS_REAL_E2E_V4

Local integration proof only. No public Base Sepolia transaction, production deployment, or GitHub push occurred.

## Reproduction

From the backend repository:

```bash
OPTIONS_V4_CHAIN_ID=84532 OPTIONS_V4_BROWSER=1 \
  bash scripts/run-options-anvil-indexer-v4.sh
```

The command checks for Anvil, Foundry, Docker, PostgreSQL client, Rust, Node, and free loopback ports. It starts a disposable Anvil chain (chain ID 84532) and PostgreSQL 16, deploys the real local Options contracts with a deterministic synthetic oracle, runs the Rust API/indexer and Next.js, then invokes Playwright Chromium. The script verifies `anvil_nodeInfo` before any Foundry broadcast. Only the local test resources it created are stopped. The test wallets and local database password are public fixture data. The Rust trade adapter lives exclusively in `tests/options_v4_live_anvil_pg.rs`; no production route or economic flag is changed.

Two isolated browser wallets sign in through the actual Next.js gateway and Rust wallet challenge. They sign the same OptionMatchingEngine EIP-712 trade with real cryptographic signatures. The test submits those signatures to the loopback Rust adapter. Rust verifies both signers, ABI-encodes `executeTrade`, submits it to local Anvil, receives the successful receipt, and indexes its logs into PostgreSQL. The authenticated portfolio renders the buyer as long one contract and seller as short one contract. A cross-wallet request returns HTTP 403; a malformed buyer signature returns 401; repeating the successful adapter request returns 409. The authenticated seller view persists across browser reload.

The Rust test compares the indexed receipt projection to `MarginEngine.getPositionQuantity` for both wallets. It advances local Anvil time, sets a local settlement price, submits two real `settleAccount` transactions, indexes both `AccountSettled` events, and confirms zero positions in the API and browser. It reverts to a snapshot taken after contract deployment and mines a replacement chain. The indexer removes the orphaned trade and settlement events, replays canonical earlier events, and has an idempotent second tick. The browser no longer displays the orphaned position. The frontend shows “No open positions” only when the empty list has `source: onchain_reconciled`; unavailable data still fails closed.

Last passing browser run (ephemeral local hashes, not public transactions):

- `REAL_ANVIL_TX=0xe8440a762d8f6cb2752499f57a9109af9e753fbc3366b5c7d79a01b267a55b05`
- `REAL_BLOCK_HASH=0xeaa3703495b92ae191c90ed5b78e1d3b6fbf56339c817d6af0d9d66d6186bf18`
- `RUST_PG_INDEXED_EVENTS=11`
- `CANONICAL_API_POSITION=buyer:long:1,seller:short:1`
- `CANONICAL_POSITION_AFTER_SETTLEMENT=buyer:0,seller:0`
- `PLAYWRIGHT_LIVE_POSITION=PASS`, `PLAYWRIGHT_SETTLED_POSITION=PASS`, `PLAYWRIGHT_REORG_POSITION_REMOVED=PASS`
- `REAL_REORG_ORPHAN_REMOVAL=PASS`; Rust integration test: `1 passed, 0 failed`.

## Acceptance boundary

This proves the local browser-signature → Rust submission → real Anvil receipt → Rust indexer → PostgreSQL → authenticated browser position path, plus local settlement and reorg recovery. The test drives its trade through the loopback test adapter rather than the production Options order form, which remains gated off. Premium, fees, collateral attribution, mark, and final balances are not fully reconciled by the current receipt projection; the API honestly reports `partial` and leaves those fields absent. The synthetic oracle does not prove Base Sepolia oracle readiness. Expired orders, insufficient collateral, invalid oracle, and double settlement require the existing contract-level tests or further integration coverage; this run does not claim those cases. It does not authorize hosted trading.
