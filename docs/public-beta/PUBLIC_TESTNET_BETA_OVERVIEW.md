# DeOpt V2 — Public Testnet Beta Overview

> **Testnet only. No real funds. Unaudited. Experimental. Not mainnet-ready. APIs and contracts may change without notice.**

---

## What DeOpt V2 is

DeOpt V2 is an experimental on-chain options protocol. It lets a buyer and a seller agree on an option trade off-chain (via signatures), then settle the trade on chain through a matching engine and a margin engine. Each trade creates or updates a position on the margin engine and routes premium + fees through the collateral vault.

The DeOpt V2 system has three repositories that compose into the testnet stack:

| Layer | Role |
|---|---|
| `deopt-v2-sol` | Solidity contracts (OptionProductRegistry, OptionMatchingEngine, MarginEngine, CollateralVault, OracleRouter, MarginEngineLens, MockPriceSource, mUSDC test token). |
| `deopt-v2-backend` | Rust / Axum service. Handles quote preview, intent creation, signing payload, EIP-712 signature collection, tx-status indexing, reconciliation, and read-only API endpoints. |
| `deopt-v2-frontend` | Next.js 16 + React 19 frontend. Wallet connect, quote ticket, signing UX, transaction-status UI. |

This documentation pack focuses on what an external tester needs to interact with the live Base Sepolia testnet deployment.

---

## What this beta covers

You can test:

1. **Wallet connection.** Connect a Base Sepolia wallet (MetaMask, Rabby, etc.).
2. **Network detection.** The frontend should detect wrong networks and prompt you to switch.
3. **Balances and portfolio.** Read your testnet ETH and mUSDC balances; view your vault deposits.
4. **Product / series exploration.** Browse the testnet option series.
5. **Quote preview.** Get a quote for a small option trade against the testnet mock oracle.
6. **Intent creation.** Create an execution intent via the public API or the frontend ticket.
7. **EIP-712 signing.** Sign the trade tuple as buyer or seller via your wallet.
8. **Trade execution.** Submit the signed trade for on-chain execution.
9. **Transaction status.** Watch the trade lifecycle from `signatures_required` → `calldata_ready` → `submitted` → `broadcast_confirmed` → `reconciled`.
10. **Position updates.** See your long / short position update on the margin engine.

The first canonical Sepolia trade has already been demonstrated end-to-end on chain (tx `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`, block `42750521`). The backend has its DB-side projection of that trade marked `reconciled` with 19 indexed events.

---

## What is NOT supported

* **No mainnet.** Base Sepolia (chain id `84532`) only. The protocol is not deployed to Base mainnet (chain id `8453`). Do not point your wallet at mainnet RPC URLs.
* **No real funds.** All tokens here are testnet mocks. The "mUSDC" collateral token is `0x6eAe407f5640B006faC9965182e238582A3B412E` on Base Sepolia and has zero real-world value.
* **No production-grade market makers.** Pricing is illustrative; do not treat any quote as financial advice.
* **No external audit.** No engagement has been started or completed.
* **No bug bounty.** Not yet. Please report issues via the feedback channels in `FEEDBACK_AND_BUG_REPORTING.md` instead.
* **No production signer.** No AWS / KMS / HSM / Safe-multisig flow is active.
* **No persistence guarantees.** The backend DB may be reset between iterations; your intent history may disappear.

---

## Current network: Base Sepolia

| Field | Value |
|---|---|
| Chain | Base Sepolia |
| Chain id | `84532` |
| Block time | ~2 s |
| Explorer | `https://sepolia.basescan.org/` |
| Public RPC | use any reliable Base Sepolia RPC provider (Alchemy, QuickNode, Infura, Ankr, etc.) |
| Faucet | `https://www.alchemy.com/faucets/base-sepolia` (or QuickNode equivalent) |

---

## Disclaimer

* **Testnet only.** Do not use real funds.
* **No real funds.** Treat all tokens as worthless test instruments.
* **Unaudited.** No external audit has been completed.
* **Experimental.** Contracts, APIs, and addresses may change without notice.
* **Not mainnet-ready.** This is not a production system.
* **Not a regulated trading venue.** No KYC/AML/MiFID compliance is provided or claimed.
* **No financial advice.** Nothing in this documentation pack constitutes financial, legal, or investment advice.

---

## Link map

* [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) — set up your wallet + get funds.
* [USER_TESTING_GUIDE.md](./USER_TESTING_GUIDE.md) — step-by-step testing flow.
* [CONTRACT_ADDRESSES_BASE_SEPOLIA.md](./CONTRACT_ADDRESSES_BASE_SEPOLIA.md) — current testnet addresses.
* [DEVELOPER_API_GUIDE.md](./DEVELOPER_API_GUIDE.md) — backend API for integrators.
* [KNOWN_LIMITATIONS_AND_RISKS.md](./KNOWN_LIMITATIONS_AND_RISKS.md) — caveats and risks.
* [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md) — how to send us feedback.
* [FAQ.md](./FAQ.md) — common questions.

---

**End of public-testnet-beta overview.**
