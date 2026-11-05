# DEOPT_OPTIONS_ORACLE_ECONOMIC_SAFETY_V6

Date: 2026-11-05. Scope: local Solidity, Rust, PostgreSQL 16, Anvil chain ID 84532 and Chromium. No Base Sepolia transaction, hosted deployment, production database change or GitHub push.

## Oracle emergency policy

| Operation | Local policy and contract evidence |
| --- | --- |
| Open, increase, negotiated reduction or early close | `OptionMatchingEngine.executeTrade` and `executeRfqTrade` require a positive fresh price from the configured oracle. Every trade is blocked during an outage. A signed opposite trade is not inherently risk reducing: it also transfers premium and fees and can leave the opposite side less solvent. No bypass is provided. |
| Cancel | `cancelNextNonce` and `cancelNoncesUpTo` change only the trader's nonce, without an oracle read. The on-chain cancellation path remains available during an outage. |
| Deposit | `CollateralVault.deposit` remains available if its own ingress controls allow the token. |
| Withdraw | The local `CollateralVault._withdrawInternal` now applies the MarginEngine's risk preview and checks a fresh oracle price for every open series. It covers direct `withdraw`, internal-account withdrawals, and `withdrawFor`; failed calls are rejected. A fallback maintenance floor is never treated as proof that funds may leave. This is a contract change, not deployed on Base Sepolia. |
| Expiry, exercise, settlement | Time can pass during an outage. `MarginEngine.settleAccount` refuses to settle until `OptionProductRegistry` has a nonzero finalized settlement price. `setSettlementPrice` is restricted to owner or settlement operator, but the contract does not prove the source of that price. The registry's constructor defaults `settlementFinalityDelay` to zero. An emergency price therefore requires a separately approved evidence and governance procedure; no automatic last-price fallback is acceptable. |
| Liquidation | `MarginEngineOps` obtains a safe oracle read and rejects absent or stale prices. Positions may remain open while feeds are down. |

The delayed-settlement option already exists in the registry, but its configured value and privileged operator must be verified on the deployed contracts before any testnet economic release. Prolonged outages require a governed reference price or a pause until reliable feeds return. The local test proves the mechanical stop and later privileged settlement; it does not validate a live emergency reference price.

## Economic accounting

The local Anvil fixture deploys and wires the real `FeesManagerV2` to `MarginEngine`, using its OPTION tier-0 schedule: maker 50 ppm and taker 250 ppm. The fee recipient is a separate local account. Rust indexes `FeeChargedV2` and `CollateralVault.InternalTransfer`. The authenticated positions API emits `fee_proof` only for a single trade in the transaction whose fee event, configured fee-manager address, settlement asset, wallet, premium basis, recipient, block hash and unique vault transfer agree. The Next.js position table shows that verified amount; otherwise it shows `unverified`. The API remains `partial` because a verified fee does not prove per-position margin or an executable mark.

The benchmark checks the buyer, seller and fee-recipient vault balances before and after the trade, then reconciles the actual settlement transfers and final balances. The local mock settlement token has six decimals; contract native units are never mislabeled as ETH or canonical USDC. The vault stores collateral per account, while the RiskModule computes aggregate account margin. No contract ledger assigns a unique locked amount to each option position. Per-position collateral allocation must therefore remain **PARTIAL**. A single-fill realized cash flow can be derived in the test from the premium transfer, fee transfers and settlement receipts; a general API PnL projection for multiple fills remains unverified.

Final V6 run: real Anvil transaction `0xfcba58f809caf8910273a70df5cd27cbb9e7cf9a47749e90ed1db0068354af4a` in block `0x3dbb0ca282bc5e387330413ea9d884d8a9b449293b4ff28da4654beec9864435`; 27 events indexed in PostgreSQL. The fixture transferred a 100,000,000 native-unit premium, charged 25,000 buyer and 5,000 seller native units, and rejected an excessive direct vault withdrawal against the actual RiskModule. Following settlement, the test measured buyer cashflow +399,975,000 and seller cashflow -400,005,000 native units, with full three-account vault conservation. Chromium showed both verified fees and the resulting positions. Double settlement, invalid/stale oracles and insufficient collateral were rejected by the real local contracts. The reorg test removed an orphaned position from the browser. These are local synthetic-token results, not live economic data.

## Reproduction and evidence

```bash
cd /home/corio/DEOPT/deopt-v2-backend
OPTIONS_V4_CHAIN_ID=84532 OPTIONS_V4_BROWSER=1 OPTIONS_V5_PRODUCT=1 bash scripts/run-options-anvil-indexer-v4.sh

cd /home/corio/DEOPT/deopt-v2-sol
forge test --match-path test/scenario/options/OptionSettlementFlow.t.sol --match-test testOracleOutageBlocksWithdrawalUntilGovernedSettlement -vv
forge test --match-path test/unit/vault/CollateralVault.t.sol -q
forge test --match-path test/unit/margin/MarginEngine.t.sol -q

cd /home/corio/DEOPT/deopt-v2-frontend
npm run typecheck
```

The launcher binds only loopback services, verifies `anvil_nodeInfo`, uses a disposable PostgreSQL container, and cleans only resources it created. Its synthetic oracle and test wallets are local fixtures. A local broadcaster gas cap of 1,000,000 became insufficient after enabling real fees: observed estimate 822,655 and Rust's 125% safety threshold 1,028,318. The benchmark cap is now 1,500,000. This is not a production gas configuration change.

## Base Sepolia release gate

The guarded `OptionMatchingEngine` and guarded `CollateralVault` are local source changes. The existing on-chain contracts do not gain them without deployment and migration or a separately validated equivalent. Before a first testnet trade: verify current contract code and addresses, the registry's Call series and any proposed Put, fee-manager wiring, vault risk configuration, oracle freshness and divergence at one canonical block, settlement operator/finality policy, executor permissions, and the gas cap. Plan preservation of existing vault balances and a controlled pause/rollback before any replacement. The Options indexer and read routes would require a separately authorized Netcup/Vercel deployment; production writes remain disabled. No Perps V10, D4/D6, or governance state is changed here.

## Verdicts

| Gate | Verdict | Evidence / remaining condition |
| --- | --- | --- |
| ORACLE_EMERGENCY_POLICY | PARTIAL | Local no-price controls and cancellation path established; live governed settlement reference not approved. |
| SAFE_POSITION_REDUCTION | BLOCKED | Current trade ingress rejects all reductions during outage; a safe alternative is not proven. |
| EMERGENCY_SETTLEMENT_POLICY | PARTIAL | Finalized price required; price-source governance and nonzero finality delay pending. |
| ORACLE_FAILURE_TESTS | PASS (local) | Outage, nonce cancellation, withdrawal refusal, expiry refusal and governed recovery in Foundry; invalid/stale-price contract checks in Anvil. Live-chain emergency policy remains a separate gate. |
| OPTIONS_FEE_ACCOUNTING | PASS (local) | Real FeesManagerV2, `FeeChargedV2` and unique vault transfer per fee; buyer 25,000 and seller 5,000 native units shown in Chromium. |
| COLLATERAL_ALLOCATION | PARTIAL | Account-level margin only; no per-position locked ledger. |
| PREMIUM_RECONCILIATION | PASS (local) | Buyer pays and seller receives 100,000,000 native units, verified against vault balances and indexed receipt. |
| REALIZED_PNL | PARTIAL | Single-fill test cash flow only; general multi-fill API projection pending. |
| ECONOMIC_INVARIANTS | PASS (local) | Vault balance conservation includes premium, fees, recipient and settlement; double settlement and insufficient collateral rejected. |
| FULL_PRODUCT_E2E | PASS (local) | V5 launcher executes Chromium, Rust, Anvil, PostgreSQL, settlement, reorg and browser position display; final V6 run passed in 54.45 seconds. |
| BASE_SEPOLIA_CONTRACT_COMPATIBILITY | BLOCKED | Local changes not deployed; current on-chain code/config requires fresh preflight. |
| REQUIRED_ONCHAIN_CHANGES | BLOCKED | Guarded vault/matching deployment or equivalent, safe state migration, settlement governance and fee wiring need separate approval. |
| GIT_STATUS | LOCAL_ONLY | Validated changes committed in all three repositories; worktrees clean, no push. |

Recommendation: **NO-GO** for opening Options on Base Sepolia until the vault withdrawal guard and settlement-price governance are reviewed and safely deployable. The next operation is a focused contract deployment and state-preservation review of those two gates, followed by an explicit operator decision. No on-chain action is authorized by this report.
