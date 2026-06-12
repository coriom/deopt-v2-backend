# DeOpt V2 — Contract Addresses (Base Sepolia)

> **Testnet only. No real funds. Unaudited. Experimental. Addresses subject to change without notice. Not mainnet.**

Chain id: **`84532`** (Base Sepolia). All addresses below live on Base Sepolia. None of these contracts exist on Base mainnet (`8453`). Do not point your wallet at mainnet.

---

## Core protocol contracts (canonical retargeted pair)

| Contract | Address | Notes |
|---|---|---|
| `OptionMatchingEngine` | **`0x5a5EBF9A9CCd7c012518569DE8283982982670f6`** | EIP-712 verifying contract; `executeTrade` entry point. |
| `MarginEngine` | **`0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`** | Position bookkeeping; `applyTrade` is callable only by the wired matching engine. |
| `OptionProductRegistry` | `0x3d52b033fab00ed6104dd3bc0a715f8648344eca` | Series catalogue; `seriesAt(i)` walks the series list. |
| `CollateralVault` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` | Holds buyer / seller deposits; receives premium + fee transfers. |
| `CollateralVaultViews` | `0x00340C360353a5AB784c5Bc5c44322A6AF0625D3` | Same address — the views surface is inherited into the concrete vault. |
| `OracleRouter` | `0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581` | Routes `getPriceSafe(under, settle)` to the right price source. |
| `MarginEngineLens` | `0x496A57CF4e0d4F1BC5c00969Ed4C5204072ddA26` | Stateless read-only helper for account / position previews. |

The pair `OptionMatchingEngine` ↔ `MarginEngine` is **bidirectionally wired**:

* `OptionMatchingEngine.marginEngine()` returns `0x506cD65a…`.
* `MarginEngine.matchingEngine()` returns `0x5a5EBF9A…`.

A legacy / stale matching engine (`0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b`) and a legacy margin engine (`0x287Cef479be5889eEfCa847F9e73C860898f48Cc`) also exist on chain but are NOT the canonical pair. Do not use them for new trades.

---

## Test collateral token

| Contract | Address | Notes |
|---|---|---|
| **mUSDC** (test) | **`0x6eAe407f5640B006faC9965182e238582A3B412E`** | 6 decimals. Mock ERC-20. Owner-mintable. Zero real-world value. |

---

## Test underlying (for the current canonical series)

| Address | Role |
|---|---|
| `0x4DeEBc5f537F3b8ba0E3393807B4D699D72bDd02` | Mock "ETH-like" underlying for series #0 (call option, strike `$3000`, expiry `2030-01-01`). |

---

## Oracle mock sources (for series #0)

These are the `MockPriceSource` contracts the OracleRouter routes to for the canonical series. They are owner-controlled; the operator pushes a fresh price via `setPrice(uint256)` immediately before any trade because the feed has a tight `maxDelay = 60 s`.

| Source | Address |
|---|---|
| Primary | `0x3eb9cdd2C2115c3f0DF5E30da53D7245F9a5f6Cc` |
| Secondary | `0x2103a84C0CAB9cf7680d602C8931FaDeD7064517` |

These are public read-only on chain. You can call `getLatestPrice()` to inspect their state. You cannot push a price unless you are the source owner.

---

## Public testnet EOAs

| Role | Address |
|---|---|
| Executor (broadcaster of `executeTrade`) | `0x295005fd4F311e6691F008D57d32FCFEde844518` |
| Demo buyer | `0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC` |
| Demo seller | `0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88` |

The demo buyer + seller are pre-funded with mUSDC and have a small testnet ETH balance. They were used by the team to demonstrate the first end-to-end Sepolia option execution.

External testers should use their own wallets, not these addresses.

---

## Canonical reference trade

The first successful Base Sepolia option execution is on chain:

```
tx hash       = 0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a
block         = 42750521
status        = 1 (success)
gas used      = 683_044
events        = 19
event name    = OptionTradeExecuted (emitted by 0x5a5EBF9A…)
```

Open it on Basescan: `https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a`.

---

## How to verify these addresses yourself

Each address above can be verified read-only via `cast`:

```bash
RPC=<your Base Sepolia RPC URL>

# Confirm the chain
cast chain-id --rpc-url $RPC                                  # → 84532

# Wiring check
cast call 0x5a5EBF9A9CCd7c012518569DE8283982982670f6 \
  "marginEngine()(address)" --rpc-url $RPC
# → 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30

cast call 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30 \
  "matchingEngine()(address)" --rpc-url $RPC
# → 0x5a5EBF9A9CCd7c012518569DE8283982982670f6

# Series catalogue
cast call 0x3d52b033fab00ed6104dd3bc0a715f8648344eca \
  "totalSeries()(uint256)" --rpc-url $RPC

# mUSDC properties
cast call 0x6eAe407f5640B006faC9965182e238582A3B412E "symbol()(string)" --rpc-url $RPC
cast call 0x6eAe407f5640B006faC9965182e238582A3B412E "decimals()(uint8)" --rpc-url $RPC
```

Do NOT use mainnet RPC URLs in these commands. The contracts at the same byte addresses on Base mainnet are NOT DeOpt V2 — they would be unrelated mainnet contracts or simply nothing.

---

## Disclaimer

* **Testnet only.** All addresses above live on Base Sepolia (chain id `84532`).
* **Subject to change.** The team may redeploy or re-wire any of these contracts without notice.
* **No mainnet.** No DeOpt V2 deployment exists on Base mainnet.
* **Not audited.** Do not infer any safety property from the fact that these addresses exist on chain.

---

**End of contract addresses (Base Sepolia).**
