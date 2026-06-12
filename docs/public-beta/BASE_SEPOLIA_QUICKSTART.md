# DeOpt V2 — Base Sepolia Quickstart

> **Testnet only. No real funds. Unaudited. Experimental.** Five-minute setup to get a wallet ready for the DeOpt V2 public beta.

---

## 1. Switch your wallet to Base Sepolia

Base Sepolia parameters:

| Field | Value |
|---|---|
| Network name | Base Sepolia |
| Chain id | `84532` |
| Currency symbol | ETH (testnet — zero real value) |
| Block explorer | `https://sepolia.basescan.org/` |
| Public RPC | a reliable Base Sepolia RPC provider of your choice |

Most modern wallets (MetaMask, Rabby, Frame, Coinbase Wallet, …) ship with Base Sepolia preconfigured. If yours does not, add a custom network with the parameters above. **Use a Base Sepolia RPC provider you trust — do not reuse mainnet RPC URLs.**

---

## 2. Get Base Sepolia ETH

You need a small amount of testnet ETH to pay gas. Recommended minimum: **`0.01` testnet ETH** per wallet. Actual usage is much lower at typical Base Sepolia gas prices (~`0.006` gwei), but `0.01` ETH leaves a comfortable buffer.

Recommended faucets (operator-side; pick whichever works for you):

* Alchemy: `https://www.alchemy.com/faucets/base-sepolia`
* QuickNode: `https://faucet.quicknode.com/base/sepolia`
* Coinbase: `https://faucet.quicknode.com/base/sepolia`
* Other community faucets — search "Base Sepolia faucet" and pick a reputable provider.

> ⚠️ Some faucets require a small mainnet ETH balance to discourage abuse. This is the faucet operator's policy, not DeOpt's.

---

## 3. Get testnet mUSDC

The DeOpt V2 testnet uses a mock USDC token ("mUSDC") at:

```
0x6eAe407f5640B006faC9965182e238582A3B412E   (Base Sepolia)
```

mUSDC has **`6` decimals** (same as real USDC). It has **zero real-world value**.

### Option A — request from the operator

Open a ticket or DM in one of the feedback channels listed in [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md) and ask for a mUSDC mint. Include your Base Sepolia wallet address. The operator will mint a small amount (e.g. `1_000` mUSDC = `1_000_000_000` raw units) for you.

### Option B — wait for the public faucet

The team is working on a public testnet mUSDC faucet. Watch the feedback channels for the announcement.

---

## 4. Open the app

```
APP_URL: {{ app deployment URL — operator to fill before announcing the beta }}
```

(If you reached these docs from a public link, the same site should host the app under a `/app` subpath or a sibling domain.)

Once the app loads:

1. Click **Connect Wallet**.
2. The app should detect that you are on Base Sepolia. If your wallet is on a different network, switch to Base Sepolia.
3. The app should display your wallet address, your testnet ETH balance, and your mUSDC balance.

---

## 5. Run a sample option trade

Follow [USER_TESTING_GUIDE.md](./USER_TESTING_GUIDE.md) for the full 10-step flow. The short version:

1. Browse the available option series (currently a single series: call on the mock ETH-like underlying `0x4DeEBc5f…`, strike $3000, expiry far in the future, mUSDC settlement).
2. Click **Preview Quote** for a small quantity (1 contract).
3. Click **Create Intent**.
4. Sign the EIP-712 typed data as buyer or seller via your wallet.
5. (If you are testing both sides yourself, sign as the other party too.)
6. Wait for the trade to be **broadcast_confirmed** and **reconciled**.
7. Open the Portfolio page and see your new position (`+1` long or `-1` short).

---

## 6. Report bugs

See [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md). Useful fields to include in your report:

* wallet address (Base Sepolia)
* timestamp
* tx hash (if any)
* network (`Base Sepolia` / chain id `84532`)
* browser + wallet
* steps to reproduce
* expected vs actual behavior
* screenshot if it's a UI bug

**Never share your private key or seed phrase.** Not in a bug report, not in a Discord DM, not anywhere. The DeOpt team will never ask for it.

---

## 7. Troubleshooting

| Symptom | Likely cause | What to do |
|---|---|---|
| App says "wrong network" | Wallet not on Base Sepolia (chain id `84532`) | Switch network in your wallet; reload the app. |
| Wallet rejected the signature | You declined the signing prompt, or the wallet popup was closed | Click **Sign** again; do NOT re-broadcast the underlying trade unless the app prompts you. |
| Quote preview returns "stale oracle" | The mock oracle's last `setPrice` is older than `60 s` | This is a known testnet caveat. Wait for the next operator refresh, or try again. |
| Quote preview returns "no quote" | The series might be inactive, or the input quantity is too large | Pick a smaller quantity or a different series. |
| Quote preview returns "source unavailable" | One of the upstream contracts is temporarily unreachable | Retry; if persistent, report it in the feedback channel. |
| "insufficient balance" | You don't have enough mUSDC to cover the premium + collateral | Request more mUSDC; see step 3 above. |
| "insufficient allowance" | You haven't approved the CollateralVault to pull mUSDC | The app should prompt for an approval transaction first; complete it. |
| Tx failed / reverted | Many possible causes (oracle stale at broadcast time, balance changed, nonce mismatch, …) | Open the tx hash on Basescan to see the revert reason; share it in your bug report. |

---

## 8. What this quickstart does NOT cover

* Operator-only flows: oracle refresh, executor authorization, lens deployment, governance.
* Mainnet anything.
* Production-grade key management.
* Real-money settlement.

If you want to learn about those, the internal docs in `~/DEOPT/deopt-v2-backend/docs/SEPOLIA_*` cover them — but they are not the public-beta surface.

---

**End of Base Sepolia quickstart.**
