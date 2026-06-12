# DeOpt V2 — Community Onboarding (Public Testnet Beta)

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited. Experimental.** This document onboards external community members into the DeOpt V2 testnet beta and feedback program.

If you're reading this, welcome — you've found an early-access experiment. Please go in with the right expectations: this is a sandbox, not a launch.

---

## 1. What DeOpt V2 is

DeOpt V2 is an **on-chain options protocol**. A buyer and a seller agree off-chain (via EIP-712 typed-data signatures) on a trade; the matching engine then settles it on chain. The protocol runs on Base Sepolia testnet for now.

Three repositories make up the stack:

* `deopt-v2-sol` — Solidity contracts (matching engine, margin engine, collateral vault, registry, oracle router).
* `deopt-v2-backend` — Rust / Axum backend (read APIs, executor service, indexer).
* `deopt-v2-frontend` — Next.js 16 / React 19 trading UI.

See [PUBLIC_TESTNET_BETA_OVERVIEW.md](./PUBLIC_TESTNET_BETA_OVERVIEW.md) for the full breakdown.

---

## 2. How to join the beta

1. **Read the public-beta docs pack first.** Start with [PUBLIC_TESTNET_BETA_OVERVIEW.md](./PUBLIC_TESTNET_BETA_OVERVIEW.md), then [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md).
2. **Connect a wallet to Base Sepolia.** Any EVM wallet supporting EIP-712 typed-data signing: MetaMask, Rabby, Frame, Coinbase Wallet, Trust, Phantom (EVM mode).
3. **Get testnet ETH.** Use any reputable Base Sepolia faucet (Alchemy, QuickNode, Coinbase). Tiny amount — `0.01 ETH` is plenty.
4. **Get test mUSDC.** Either request from the operator via the public feedback channel, or wait for the public mUSDC faucet (in the roadmap).
5. **Open the app, run a sample trade.** Walk through the [USER_TESTING_GUIDE.md](./USER_TESTING_GUIDE.md) 10-step flow.
6. **Tell us what broke.** See "How to report bugs" below.

You do not need an invite. You do not need to KYC. You do not need to buy anything.

---

## 3. What testers can try

* Browse option products and series.
* Get a quote preview for a series + side + size.
* Create an execution intent on the backend.
* Sign the EIP-712 typed-data payload in your wallet.
* Submit your side of the signature to the backend (the executor broadcasts; you don't).
* Watch the trade-status timeline cycle from `CREATED` → `BROADCAST` → `CONFIRMED`.
* View your testnet positions, balances, portfolio summary, trade history.
* Read the API docs and the worked viem-style integration example.
* Self-host the backend or frontend if you want to dig deeper.

Canonical first-trade reference (for what a healthy lifecycle looks like):

```
tx_hash      = 0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a
block        = 42750521
status       = 1 (success)
gas_used     = 683_044
events       = 19
explorer     = https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a
```

You don't need to reproduce this trade — it's just there as a reference for what success looks like.

---

## 4. What testers should NOT do

* **Do not deposit real funds.** All tokens in this beta are testnet mocks (mUSDC). They are worthless.
* **Do not connect a mainnet wallet to this app.** The app refuses mainnet, but please don't try to bypass the gate.
* **Do not share private keys or seed phrases.** With anyone, ever. No one from DeOpt will ever ask for them. If anyone in the community asks you for these claiming to represent DeOpt, they are an impostor — block + report.
* **Do not assume this is production.** APIs may change without notice; contracts may be redeployed; the database may be reset; downtime is expected.
* **Do not claim DeOpt is audited / mainnet-live / safe for real funds** in any external communication. We are NOT those things and we will be very direct in correcting that wherever we see it.
* **Do not try to game the bounty.** There is no formal bug-bounty yet; please report security issues anyway via the private path (see §6) — appreciation > monetisation right now.

---

## 5. Getting testnet ETH + mUSDC

### Testnet ETH

Any Base Sepolia faucet works. Recommended:
* Alchemy Base Sepolia faucet
* QuickNode Base Sepolia faucet
* Coinbase Wallet's built-in testnet faucet

Tiny amount — `0.01 ETH` is plenty for many trades. You only need testnet ETH for gas; the executor key pays its own gas for the actual `executeTrade` broadcast.

### Test mUSDC

Two paths, in order of speed:

1. **Operator-mint.** Ask the operator in the public feedback channel to mint some mUSDC to your wallet address. Usually quickest right now.
2. **Public mUSDC faucet.** In the roadmap; not live yet.

mUSDC details:
* address: `0x6eAe407f5640B006faC9965182e238582A3B412E`
* decimals: 6
* 1 mUSDC = `1_000_000` raw units (matches real USDC semantics)

See [BASE_SEPOLIA_QUICKSTART.md §3](./BASE_SEPOLIA_QUICKSTART.md) for the latest mUSDC instructions.

---

## 6. How to report bugs

* **Public bugs (most things).** Use the [BUG_REPORT_TEMPLATE.md](./BUG_REPORT_TEMPLATE.md). Post in GitHub issues, Discord, Telegram, or the feedback form per [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md).
* **Security-impacting bugs.** Use the **private** GitHub Security Advisory path, or DM the maintainer team. Do NOT post specifics in public channels. See [FEEDBACK_TRIAGE_WORKFLOW.md §6](./FEEDBACK_TRIAGE_WORKFLOW.md).

The bug-report template includes a safety checklist — please run through it before submitting.

---

## 7. What feedback is most valuable

In rough order of usefulness:

1. **Reproducible reverts.** Any `executeTrade` tx that reverted unexpectedly. Tx hash + intent id + steps.
2. **Wallet-specific failures.** "Wallet X version Y can't sign DeOpt typed data" — please tell us which wallet, which version, which failure.
3. **Confusing UX.** Even if you can't articulate why — "I got confused at step 4" is a useful report.
4. **Backend / API ergonomics.** If you're integrating against the API and the OpenAPI spec is wrong or missing a field, that's high-priority.
5. **Docs that mislead.** If a doc made you do the wrong thing, we want to know exactly which doc + which section.
6. **Stale-oracle false positives.** If the UI says "stale oracle" when the oracle actually was fresh — please share the timestamp.

What's less helpful (we still read it, but lower urgency):
* "It would be cool if you added X" without context — fine, but please file as `feature-request`.
* Pure styling nits unless they affect comprehension.

---

## 8. Disclaimers — read once, internalise

* **Testnet only.** Base Sepolia (chain id `84532`). Mainnet (Base mainnet, chain id `8453`) is NOT supported.
* **No real funds.** All tokens are mocks. mUSDC has zero real-world value. There is no economic exposure.
* **Unaudited.** No external audit has happened. A security-review packet is being prepared internally — that packet is NOT itself an audit.
* **Experimental.** APIs, contracts, addresses, and behaviour may change without notice.
* **Not mainnet-ready.** Don't position this as something you can deploy on mainnet today. It's not.
* **Feedback phase.** This entire beta exists to gather feedback. The protocol's job right now is to be useful enough to test, not to be useful enough to trade against.
* **Community preview.** You're seeing it early. That comes with rough edges. Thank you for tolerating them.

---

## 9. Operator promises (and non-promises)

We **promise**:
* Every bug report is read.
* Security disclosures get a private response within 1 business day.
* Pause / reset events are announced honestly in all public channels.
* No one from the operator team will ever ask you for a private key or seed phrase.

We **do not promise**:
* A fix for every bug.
* SLA uptime.
* A bug bounty (not active yet; may be later).
* That contract addresses will stay stable through the beta (they may be redeployed).
* That the backend database won't be reset (it may be).

---

## 10. One last thing

You are welcome here. Test things. Break things. Tell us what hurts. Be kind to other testers. Don't take the protocol's polish-level as a judgement of you — it's just a beta.

See you in the channels.

---

**End of community onboarding.**
