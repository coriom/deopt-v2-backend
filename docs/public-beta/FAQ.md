# DeOpt V2 — Frequently Asked Questions (Public Beta)

> **Testnet only. No real funds. Unaudited.** Common questions, answered.

---

### What is DeOpt V2?

An experimental on-chain options protocol. A buyer and a seller agree off-chain (via EIP-712 signatures) on a trade; the matching engine then settles it on chain. The protocol runs against Base Sepolia testnet for now. See [PUBLIC_TESTNET_BETA_OVERVIEW.md](./PUBLIC_TESTNET_BETA_OVERVIEW.md).

---

### Is this on mainnet?

**No.** Base Sepolia (chain id `84532`) only. There is no mainnet deployment. If anyone tells you DeOpt V2 is live on mainnet, they are mistaken or impersonating us. See [KNOWN_LIMITATIONS_AND_RISKS.md](./KNOWN_LIMITATIONS_AND_RISKS.md).

---

### Is this audited?

**No.** No external audit has been completed or even started. A security-review packet is being prepared internally as a precursor; that itself is not an audit. See `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`.

---

### Is there a bug bounty?

Not yet. Please report bugs through the channels in [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md). A formal bounty program may be launched later; the scope and rules will be published explicitly when it does.

---

### Can I make money on this?

**No.** All tokens are testnet mocks. mUSDC has zero real-world value. The premium amounts you see are illustrative. Do not try to "trade" testnet positions off-platform expecting real value.

---

### Do I need to KYC?

No. The testnet beta does not collect KYC information. We do not provide compliance services. If your local jurisdiction restricts experimental DeFi participation, please consult your own counsel before using the beta.

---

### Which wallets work?

Any EVM wallet that supports Base Sepolia and EIP-712 typed-data signing:

* MetaMask
* Rabby
* Frame
* Coinbase Wallet
* Trust Wallet (recent versions)
* Phantom (with EVM mode enabled)

If your wallet doesn't support typed-data signing, the signing step in [USER_TESTING_GUIDE.md](./USER_TESTING_GUIDE.md) step 6 will fail. Try a different wallet.

---

### Where do I get testnet ETH?

Use any reputable Base Sepolia faucet. See [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) §2 for recommendations.

---

### Where do I get testnet mUSDC?

Two paths:

1. Request from the operator via the feedback channel (fastest right now).
2. Wait for the public mUSDC faucet to launch (in the roadmap).

Details in [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) §3.

---

### Why does my quote say "stale oracle"?

The testnet mock oracle has a tight `60 s` freshness window. If no `setPrice` has been pushed recently, the OracleRouter returns `ok=false` and the backend marks the quote `partial` with an `ORACLE_UNAVAILABLE` warning.

* If you're a tester: wait a moment and retry, or ping the operator in the feedback channel.
* If you're a developer: handle `status: "partial"` gracefully and don't auto-submit signatures against a stale quote — the resulting tx will revert on chain.

---

### Why did my transaction revert?

Most common causes on testnet:

| Reason | What to check |
|---|---|
| Stale oracle at broadcast time | `getPriceSafe(under, settle)` returns `(0, 0, false)`. Operator needs to refresh setPrice. |
| Wrong matching engine | Signatures targeted the stale ME `0xf2D1D85…` instead of the canonical `0x5a5EBF9A…`. |
| Insufficient collateral | Seller doesn't have enough mUSDC deposited in the vault to cover the short. |
| Insufficient allowance | Buyer or seller hasn't approved the vault as spender. |
| Nonce mismatch | The nonce in your signed tuple doesn't match the current on-chain nonce. (Usually means someone else's trade landed first.) |
| Deadline expired | Your `deadline` was set too short; the executor took too long to broadcast. |
| `NotAuthorized()` (`0xea8e4eb5`) | The matching engine isn't authorized on the downstream margin engine. This was the bug fixed by the `MatchingEngine` retarget on `2026-06-12`; should not recur on the canonical pair. |

Open the tx on Basescan to see the revert reason, and share the hash in your bug report.

---

### Why did I sign two transactions to start trading?

You probably signed:

1. **ERC-20 approve** — letting the CollateralVault pull mUSDC from your wallet (gas tx on chain).
2. **EIP-712 typed-data signature** — the actual trade payload (no gas, off-chain).

If you also needed to deposit mUSDC into the vault, that's a third tx. After the first deposit, subsequent trades only need the typed-data signature.

---

### Why does the executor broadcast `executeTrade`, not my wallet?

DeOpt V2's matching engine is designed for a **two-sided signature** flow: buyer signs the trade tuple, seller signs the trade tuple, and a third party (the executor) submits both signatures + the tuple to `executeTrade`. The executor pays gas and is rate-limited; buyer and seller don't need to be online at broadcast time.

This is the standard EIP-712-style flow used by Uniswap X, CoW Swap, and similar protocols.

---

### What's the difference between `intent_id` and `onchain_intent_id`?

* `intent_id` — a UUID assigned by the backend. Used in API paths (`/options/execution-intents/:intent_id/...`).
* `onchain_intent_id` — a 32-byte hash baked into the EIP-712 typed data. Visible on chain in the `OptionTradeExecuted` event's first topic.

They are not interchangeable. The backend joins them in the `option_execution_intents` table.

---

### What if the indexer falls behind?

Symptoms: lifecycle endpoint shows `reconciliation.status: missing_events` even after the tx confirms on chain.

* Wait ~30 s; the worker ticks periodically.
* If it persists, share the tx hash in the feedback channel; we'll force a tick.

---

### Can I run my own backend?

Yes. The source is in `deopt-v2-backend/`. You'll need:

* Rust nightly (or stable that matches the `rust-toolchain.toml`).
* Postgres ≥ 14.
* A Base Sepolia RPC URL.
* The retargeted contract addresses (in [CONTRACT_ADDRESSES_BASE_SEPOLIA.md](./CONTRACT_ADDRESSES_BASE_SEPOLIA.md)).

`cargo build --release` then run with the right env vars. Do NOT enable `EXECUTOR_REAL_BROADCAST_ENABLED=true` unless you understand the executor key handling.

---

### Can I run my own frontend?

Yes. The source is in `deopt-v2-frontend/`. `npm install` + `npm run dev`. Point at your backend's API URL.

---

### Why is the legacy matching engine `0xf2D1D85…` still on chain?

Historical. It was the original deployment; a retarget on `2026-06-12` moved the canonical flow to `0x5a5EBF9A…`. The legacy contract is harmless (cannot call the downstream margin engine), but it should NOT be used for new trades. See [CONTRACT_ADDRESSES_BASE_SEPOLIA.md](./CONTRACT_ADDRESSES_BASE_SEPOLIA.md).

---

### What's the canonical first Sepolia trade?

```
tx_hash      = 0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a
block        = 42750521
status       = 1 (success)
gas_used     = 683_044
events       = 19
```

Open it on Basescan: `https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`.

You don't need to reproduce this exact trade — it's just a reference example so you know what a healthy lifecycle looks like.

---

### What's the roadmap?

Next public milestones (subject to change):

1. **Frontend testnet polish.** Wrong-network handling, testnet banners, copy improvements.
2. **Community feedback loop.** Wiring up the GitHub / Discord / Telegram channels.
3. **Product freeze + security-review packet.** Re-anchoring the frozen ABI and writing the public security packet.

Mainnet, external audit, and bug bounty come AFTER those, and only after the security-review packet has been independently reviewed.

---

### How do I contact the team?

See [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md). Quick links once placeholders are filled in:

* GitHub: `{{ GITHUB_REPO_URL }}`
* Discord: `{{ DISCORD_INVITE_URL }}`
* Telegram: `{{ TELEGRAM_INVITE_URL }}`
* Feedback form: `{{ FEEDBACK_FORM_URL }}`

---

**End of FAQ.**
