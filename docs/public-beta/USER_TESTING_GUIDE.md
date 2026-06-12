# DeOpt V2 — User Testing Guide

> **Testnet only. No real funds. Unaudited. Experimental.** This guide walks an external tester through one full option-trading lifecycle on Base Sepolia.

Prerequisite: complete the [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) (wallet on Base Sepolia, ≥ `0.01` testnet ETH, some testnet mUSDC).

---

## What we'll test

| Step | What | Why it matters |
|---|---|---|
| 1 | Connect wallet | Confirms the wallet → app handshake. |
| 2 | Check balances | Confirms the read path against the CollateralVault. |
| 3 | View product / series | Confirms the read path against the OptionProductRegistry. |
| 4 | Preview quote | Confirms the oracle + pricing read path. |
| 5 | Create execution intent | Confirms the backend intent-creation endpoint. |
| 6 | Sign EIP-712 typed data | Confirms the signing payload + your wallet's typed-data UX. |
| 7 | Submit trade | Confirms the executor's broadcast path (broadcast happens off the trader's wallet — the executor key signs). |
| 8 | Watch tx status | Confirms the lifecycle endpoint. |
| 9 | View updated position | Confirms the margin-engine read path. |
| 10 | View portfolio / balance changes | Confirms the vault / lens read path. |

---

## Step 1 — Connect wallet

1. Open the app.
2. Click **Connect Wallet** → choose your wallet (MetaMask / Rabby / etc.).
3. Approve the connection request in the wallet popup.

**Expected:** the app shows your wallet address and a "Base Sepolia" network badge. No errors.

If you see a "wrong network" banner or a "Mainnet is permanently disabled" banner, click the **Switch to Base Sepolia** button rendered inside the banner — the app uses `wallet_switchEthereumChain` to ask your wallet to flip networks. Some wallets need Base Sepolia added manually first (see [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) §1). The app intentionally does NOT add a custom RPC URL on your behalf.

You will also see a permanent **public-beta footer** at the bottom of every page with quick links to Quickstart, Testing Guide, Known Limitations, Bug reports, Discord, and GitHub. Some links may say "coming soon" until the operator wires the real channels.

---

## Step 2 — Check balances

1. Open the Account / Balances page.

**Expected:**

* **Wallet ETH balance** ≥ `0.01` testnet ETH.
* **Wallet mUSDC balance** ≥ `1` mUSDC (= `1_000_000` raw units at 6 decimals).
* **Vault deposit (mUSDC)** = whatever you've already deposited (may be `0` on your first run).

If the **Vault deposit** is `0`, the app should prompt you to:

1. Approve the CollateralVault as a spender for mUSDC.
2. Deposit some mUSDC into the vault.

Both are 1 tx each. The app should walk you through them.

---

## Step 3 — View product / series

1. Open the Markets / Products page.

**Expected:** at least one series listed. As of the public-beta launch, the canonical series is:

| Field | Value |
|---|---|
| Type | Call |
| Underlying | `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` (mock "ETH-like") |
| Settlement | mUSDC (`0x6eAe407f5640B006faC9965182e238582A3B412E`) |
| Strike | `$3000` (encoded as `300_000_000_000` in 1e8 units) |
| Contract size | `1` underlying unit (encoded as `100_000_000` in 1e8 units) |
| Expiry | `1893456000` (~`2030-01-01`) |
| Active | true |

(The app should render the underlying / strike / expiry in human-readable form.)

---

## Step 4 — Preview quote

1. Click on the series to open the ticket.
2. Select side: **Buy** or **Sell**.
3. Enter quantity: `1` contract.
4. Click **Preview Quote**.

**Expected:** the app shows a quote envelope with:

* `status: "ok"` (or `partial` with warnings)
* a premium per contract (e.g. `0.05` mUSDC for a deep ITM call — exact number depends on time + oracle freshness)
* a total premium (= quantity × premium per contract)
* zero or low warnings

**If you see `partial` with `stale oracle`:** the testnet mock oracle has a `60 s` freshness window. The operator (or the testnet refresh worker) needs to push a fresh price. Wait a moment and re-try.

---

## Step 5 — Create execution intent

1. With a valid quote on screen, click **Create Intent**.
2. The app may ask you to choose the counterparty if you're testing both sides yourself. If a market-making counterparty is available, the app should auto-pair you.

**Expected:** the app shows a new intent with status `signatures_required` and an `intent_id` (UUID). The intent contains:

* buyer + seller addresses
* series fields (mirrored)
* quantity
* premium per contract
* current matching-engine nonces for buyer + seller
* a `deadline` (typically 2 hours from now)
* an `onchain_intent_id` (a 32-byte hash)

---

## Step 6 — Sign EIP-712 typed data

1. Click **Sign Buyer** (or **Sign Seller**, depending on your role).
2. Your wallet pops up with the typed-data signing prompt. The fields should match the intent shown on screen.
3. Approve the signature.

**Expected:** the intent status advances to `calldata_ready` if both sides have signed, or stays at `signatures_required` if only one side has signed.

Key fields the wallet should show in the EIP-712 prompt:

```
Domain: { name: "DeOptV2-OptionMatchingEngine", version: "1", chainId: 84532,
          verifyingContract: 0x5a5EBF9A9CCd7c012518569DE8283982982670f6 }
Primary type: OptionTrade
Message:
  intentId: 0x…
  buyer: 0x… (your address if you are the buyer)
  seller: 0x…
  optionId: …
  underlying: 0x4DeEBc5f…
  settlementAsset: 0x6eAe407f…
  expiry: 1893456000
  strike1e8: 300000000000
  isCall: true
  contractSize1e8: 100000000
  quantity: 1
  premiumPerContract: …
  buyerIsMaker: false
  buyerNonce: …
  sellerNonce: …
  deadline: …
```

If the wallet shows a "raw signature request" (not typed data), that is a wallet-side limitation. Some wallets do not support EIP-712 prompts — try a different wallet (MetaMask, Rabby, Frame all support typed data).

---

## Step 7 — Submit trade

1. Once both buyer and seller have signed, click **Submit Trade**.
2. The app sends the signed payload to the backend. The backend's executor signs the `executeTrade` tx and broadcasts it.

**Expected:** the lifecycle status moves through `calldata_ready` → `submitted` → `broadcast_confirmed`.

The actual `executeTrade` transaction is broadcast from the **executor EOA** (`0x295005fd4F311e6691F008D57d32FCFEde844518`), not from your wallet. Your wallet only signs the trade payload off-chain.

---

## Step 8 — Watch transaction status

1. On the intent / transaction page, watch the lifecycle indicators.

**Expected:** within ~30 s of `submitted`, the page shows:

* tx hash (a real Sepolia hash, clickable to Basescan)
* `confirmation_status: mined_success`
* `confirmed_block_number` (Base Sepolia block number)
* a list of decoded events (including `OptionTradeExecuted`)

The canonical first Sepolia trade as a reference example:

```
tx_hash      = 0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a
block        = 42750521
gas_used     = 683_044
status       = 1 (success)
events       = 19  (1 OptionTradeExecuted + 1 TradeExecuted + 2 TradingFeeCharged
                    + 3 InternalTransfer + 12 Synced)
```

(You don't need to reproduce these exact numbers — they are shown here as a reference for what a successful trade looks like in the lifecycle endpoint.)

---

## Step 9 — View updated position

1. Open the Positions page.

**Expected:** your position quantity has updated by `±1` (long if you bought, short if you sold). The matching-engine nonce for your address increments by `1`.

---

## Step 10 — View portfolio / balance changes

1. Open the Account / Portfolio page.

**Expected:**

* Your vault `mUSDC` deposit has changed (buyer paid premium + fee; seller received premium minus fee).
* The fee recipient address `0x7c0a3b6F…` received the net fee.
* Your `wallet ETH` balance went down a tiny bit (you didn't broadcast `executeTrade` from your wallet, but you may have approved + deposited earlier).

---

## Common deviations and what they mean

| Deviation | What it means |
|---|---|
| Intent stays at `signatures_required` past 5 minutes | One side has not signed. Re-check the EIP-712 prompt; ask the counterparty to sign. |
| Intent moves to `broadcast_failed` | The executor tried to broadcast but the tx reverted. Check `error` for the revert reason. Most common cause on testnet: stale oracle at broadcast time. |
| Lifecycle shows `reconciliation.status = missing_events` | The indexer hasn't picked up the event yet (or the event was emitted before the indexer cursor). Give it a few seconds; retry. |
| Lifecycle shows `reconciliation.status = ambiguous` | More than one indexed event matches the intent. This shouldn't happen in normal flow; please report it. |

---

## What to report back

After running through this guide, the team would love to hear:

* Did the wallet prompt clearly show the trade details?
* Did the lifecycle status update in real time, or did you have to refresh?
* Did the portfolio update match what you expected?
* Any unclear UX copy, confusing icons, or surprising behavior?

See [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md).

---

**End of user testing guide.**
