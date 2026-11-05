# DEOPT_OPTIONS_PRODUCT_E2E_V5

Date: 2026-11-05. Scope: disposable local Anvil (chain ID 84532), PostgreSQL 16, Rust, Next.js and Playwright Chromium. No public Base Sepolia transaction, GitHub push, or hosted deployment occurred.

## Reproduction

From `deopt-v2-backend`:

```bash
OPTIONS_V4_CHAIN_ID=84532 OPTIONS_V4_BROWSER=1 OPTIONS_V5_PRODUCT=1 \
  bash scripts/run-options-anvil-indexer-v4.sh
```

The launcher requires loopback ports, proves `anvil_nodeInfo` before the Foundry broadcast, and cleans only its own Anvil and PostgreSQL test resources. `OPTIONS_V4_FRONTEND_PORT`, `OPTIONS_V4_API_PORT`, `OPTIONS_V4_ANVIL_PORT`, and `OPTIONS_V4_PG_PORT` can be set when their defaults are occupied. The oracle is a deterministic local fixture. It does not establish Base Sepolia oracle availability.

## Product path and evidence

The existing `TradeTicket` is mounted for a selected Call series in local development. The browser enters quantity, limit price and counterparty, receives an indicative quote, creates an intent through the real Rust handler, and signs with two local EIP-1193 test wallets. Rust validates both EIP-712 signatures. A separate test watcher invokes the real `/simulate` and `/broadcast` handlers after both signatures exist. During this V5 trade phase, the V4 local execution adapter routes are absent from the HTTP server. The browser never calls that adapter. Rust broadcasts the signed transaction only to the disposable Anvil RPC, indexes its receipt into PostgreSQL, checks `MarginEngine.getPositionQuantity`, and serves the authenticated portfolio. The browser displays long/short positions and the premium backed by the indexed `CollateralVault.InternalTransfer` event.

Latest passing run: `REAL_ANVIL_TX=0x05efcedfed8a021821e1b41f614033e1d55b7c51ee95c2c08d210292dbaf19d6`, `REAL_BLOCK_HASH=0x5b65e97f4fbbc27f84c24dfaf43dc5f47cecf20b9bcc88ac38f77ca623d80357`, `RUST_PG_INDEXED_EVENTS=11`, `ONCHAIN_POSITION=buyer:+1,seller:-1`, `PLAYWRIGHT_LIVE_POSITION=PASS`. These identifiers are local test artifacts, not public transactions.

The indicative quote converts 1e8 contract quantity and 1e8 price into settlement-token native units using the same configured decimals as execution. For one contract at price 100 (1e8 scale) with six-decimal mock USDC, the browser showed `100000000` native units. This quote remains `partial`: fees are an assumption, margin impact and executable price are not verified. The UI labels it indicative.

The receipt has one indexed premium vault transfer for `100000000` native units from buyer to seller, matching the actual before/after vault account balances. Seller plus buyer vault balances are conserved. The fixture has no fee manager configured, so no fee event was emitted; this proves zero local fees only. It does not establish fee correctness for a configured production fee schedule. The contracts enforce initial margin and the test withdraws seller collateral before a simulated trade to prove a real insufficient-collateral revert, but no per-position locked-collateral ledger is projected. Collateral attribution remains partial.

After local expiry, the test sets a settlement price and submits real seller and buyer `settleAccount` transactions. Indexed `AccountSettled` paid/collected amounts match same-transaction vault transfer sums and before/after vault balances. The browser shows no open position and the verified settlement amounts. A second settlement reverts. A snapshot/revert creates a replacement chain; the indexer removes orphaned trade and settlement rows and the browser removes both position and settlement proofs. An idempotent tick indexes no duplicate events.

The same run rejected: trade replay, expired EIP-712 intent after refreshing the local oracle, invalid oracle, stale oracle, insufficient seller collateral, double settlement, and cross-wallet portfolio access. The invalid-signature browser case altered a signed request before the real Rust signature handler; the handler refused it and a subsequent valid signature succeeded. The oracle checks were `eth_call` against the actual local OptionMatchingEngine after modifying the deterministic oracle. They were not Rust mocks.

## Targeted changes and boundaries

`OptionMatchingEngine` now requires the MarginEngine-configured oracle to return a positive price within its nonzero freshness limit before either signed trade or RFQ execution. The prior oracle-down margin fallback allowed a fresh trade during an oracle outage. The new guard is a local contract change and requires separate on-chain review and deployment before Base Sepolia can gain that protection. The existing Base Sepolia contracts were untouched. The compiled OptionMatchingEngine deployed bytecode is 10680 bytes, below the EIP-170 limit of 24576.

The guard also rejects risk-reducing close trades while the oracle is unavailable or its maximum delay is configured as zero. This availability tradeoff requires explicit protocol review before deployment. The local test proves refusal for new exposure, but does not prove safe emergency close behavior.

User-created Rust execution intents now expire no later than one hour or the series expiry. The local frontend uses a loopback-only economic API URL in development; production builds keep the trading UI and Rust economic execution disabled. The V5 test's local signer is an Anvil fixture; neither it nor the adapter is exposed as a production route. No production PostgreSQL migration is introduced by this change.

## Verdicts

| Control | Verdict | Evidence or limit |
| --- | --- | --- |
| REAL_OPTIONS_FORM / REAL_API_EXECUTION_PATH | PASS local | Product `TradeTicket`, real Rust create/sign/simulate/broadcast handlers; no V4 adapter route during trade |
| EIP712_BROWSER_SIGNATURE / ANVIL_TRANSACTION | PASS local | Two cryptographic signatures, receipt and block hash above |
| PREMIUM_RECONCILIATION | PASS local | Indexed transfer `100000000` native units and vault balance deltas |
| FEES_RECONCILIATION | PARTIAL | Local fixture has no fee manager; production fee schedule untested |
| COLLATERAL_RECONCILIATION | PARTIAL | Initial margin enforced and insufficient collateral rejected; no canonical per-position locked balance |
| WALLET_BALANCE_RECONCILIATION | PASS for vault accounts | Premium and settlement transfer sums match vault account balance deltas; external ERC20 wallet balances not projected |
| SETTLEMENT_ACCOUNTING | PASS for paid/collected flows | Indexed settlement events, vault transfers and balance deltas match; no full realised-PnL projection |
| POSTGRES_INDEXATION / REORG_INTEGRITY | PASS local | 11 indexed trade events; orphaned trade and settlement proofs removed after reorg |
| PLAYWRIGHT_PRODUCT_E2E | PASS local | Form to receipt to portfolio, settlement, reload and wallet isolation |
| PRODUCTION_EXECUTION_GUARD | PASS code-level | Hosted frontend disables trading; Rust production flags unchanged; V5 execution requires loopback Anvil test configuration |
| BASE_SEPOLIA_RELEASE_READINESS | BLOCKED | Updated contract not deployed; real oracle freshness, fee manager, collateral accounting and economic authorization remain separate gates |

Targeted checks: `forge test --match-path test/scenario/options/OptionSettlementFlow.t.sol` (9 passed); `cargo test --lib quote_preview -q` (9 passed); `npm run typecheck` (passed); launcher `bash -n` (passed); full command above (1 Rust integration test plus Chromium product and post-settlement/reorg runs passed). Existing repository-wide formatting and Clippy debt was not changed in this milestone.

Next action: review the local commits and the exact Base Sepolia contract/oracle/fee configuration before authorizing any on-chain deployment or hosted economic execution.
