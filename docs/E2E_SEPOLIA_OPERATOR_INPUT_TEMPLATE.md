# E2E Sepolia — Operator Input Template (M-P5 Phase A Follow-up)

**Date:** 2026-06-10
**Audience:** operator filling in private/operator-side values before
the read-only confirmation runs.
**Posture:** **template only. NEVER commit a filled-in copy. NEVER
include private keys, real RPC secrets, or AWS/KMS values in this
file.**

> **CRITICAL.** This document is a public-safe placeholder
> template. The operator fills in a PRIVATE copy in
> `~/DEOPT/operator-private/` (or equivalent untracked location). The
> filled values are NEVER checked into git.

## 1. What the operator must supply

| Field | Public-safe placeholder | Private value goes in |
|---|---|---|
| Base Sepolia RPC URL | `EXECUTION_RPC_URL=<operator-supplied>` | private `.env.sepolia` |
| OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS | _see §3 — automatically equals `OPTION_COLLATERAL_VAULT_ADDRESS` because `CollateralVaultViews` is an abstract contract inherited into `CollateralVault`_ | `.env.sepolia` |
| OPTION_MARGIN_ENGINE_LENS_ADDRESS | `<lens-deployed-on-sepolia>` | `.env.sepolia` |
| FEES_MANAGER_V2 confirmation | `<address-from-operator-deploy-notes>` | `.env.sepolia` |
| PROTOCOL_FEE_VAULT confirmation | `<address-from-operator-deploy-notes>` | `.env.sepolia` |
| Executor address | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` (public, in `~/DEOPT/TESTNET_RUNBOOK.md`) | re-confirm only |
| Test buyer address | `0xc0A76c2A6c6b70C0B065A05E64417886416cc976` (public) | re-confirm only |
| Test seller address | `0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3` (public) | re-confirm only |
| Active option series id | `<series_id-from-backend-store>` | operator picks |
| Collateral token address | `<collateral-token-on-sepolia>` | from operator notes |
| Minimum buyer ETH balance | 0.01 ETH (recommendation; see §4) | — |
| Minimum seller ETH balance | 0.01 ETH (recommendation) | — |
| Minimum seller collateral balance | `size_1e8 × strike_1e8 × bps_buffer` (operator computes) | — |
| Oracle feed status (active for chosen series) | confirm via `OracleRouter.hasActiveFeed(...)` returns `true` | — |

## 2. Forbidden

* NO private key value in this template.
* NO real RPC URL in this template.
* NO AWS account ID, KMS key ID, or KMS ARN in this template.
* NO mainnet (chain 8453) value anywhere.
* NO checked-in copy with operator data filled in.

## 3. CollateralVaultViews address derivation

`CollateralVaultViews` is an **abstract contract**
(`abstract contract CollateralVaultViews is CollateralVaultYield`).
The concrete deployed contract is `CollateralVault`
(`contract CollateralVault is CollateralVaultActions`, where
`CollateralVaultActions is CollateralVaultViews`).

The selectors the backend uses against the views surface
(`getCollateralTokens()` = `0xb58eb63f`, `balances()` = `0xc23f001f`)
appear in `selectors.txt` ONLY under the `CollateralVault` heading —
confirming the views are inherited into the concrete contract.

**Conclusion:**

```
OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS = OPTION_COLLATERAL_VAULT_ADDRESS
                                      = 0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
```

This is **anchored** to the existing
`deopt-v2-sol/docs/MARGIN_ENGINE_V2_PHASE1_BROADCAST_AUTH_PACKET_V2D_O.md`
docs (referenced in M-P5 Phase A §3.2). The operator does NOT need
to supply a separate views address.

## 4. Minimum balance recommendations

These are conservative recommendations; actual minimums depend on
the chosen series + execution gas budget:

* Executor: ≥ 0.05 testnet ETH (covers ~50 broadcast attempts).
* Buyer (long): ≥ 0.01 testnet ETH + enough settlement token to
  pay the premium.
* Seller (short): ≥ 0.01 testnet ETH + enough collateral to cover
  the short position margin requirement.

## 5. How to use this template

1. Operator creates `~/DEOPT/operator-private/sepolia-readiness-2026-06-10.md`
   (or similar untracked path).
2. Operator copies §1 into it and fills in the actual values.
3. Operator runs the read-only confirmation commands in
   `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` against the filled
   values.
4. Operator updates `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
   (public-safe summary: just status flags, no addresses or
   balances) with the read-only-confirmed status of each blocker.

## 6. Cross-links

* `E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md` (M-P5 Phase A)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md`
* `E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `E2E_SEPOLIA_BLOCKERS_AND_FIXES.md`
* `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md` (existing testnet ops runbook —
  public executor/buyer/seller addresses)

**End of operator input template.**
