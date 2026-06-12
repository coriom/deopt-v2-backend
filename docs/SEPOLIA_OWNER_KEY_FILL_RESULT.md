# SEPOLIA-OWNER-KEY-FILL — Result

**Date:** 2026-06-12
**Milestone:** Phase A + B of `SEPOLIA-OWNER-KEY-FILL-AND-SETUP-FIXES-CONTINUE`.
**Posture:** **Private key audit only. No value printed. Private file mode 600 preserved. No `.env` edit.**

---

## 1. Outcome

Operator confirmed the candidate owner key, and the audit verified
it controls the 3 Sepolia identities required for the BS pack:

| Identity | On-chain address | Status |
|---|---|---|
| `OptionMatchingEngine.owner()` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | **MATCHED** |
| `COLLATERAL_TOKEN.owner()` (mUSDC) | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | **MATCHED** |
| `MockPriceSource.owner()` (4 sources behind both series feeds) | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | **MATCHED** (all 4) |
| `TestnetMockERC20.owner()` (= COLLATERAL_TOKEN owner) | same as above | **MATCHED** |
| Address derived from `OWNER_PRIVATE_KEY` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | **MATCHED** |

The same key is also the `DEPLOYER_PRIVATE_KEY` (deployer EOA =
owner EOA). The pack runner reuses it for the BS-2 `forge create`.

---

## 2. Variable presence (no values printed)

| Variable | Shell env | Private file |
|---|---|---|
| `OWNER_PRIVATE_KEY` | PRESENT | PRESENT |
| `DEPLOYER_PRIVATE_KEY` | PRESENT | PRESENT |
| `BUYER_PRIVATE_KEY` | (not checked in shell) | PRESENT |
| `SELLER_PRIVATE_KEY` | (not checked in shell) | PRESENT |
| `CANDIDATE_OWNER_PRIVATE_KEY` | PRESENT | PRESENT |
| `BASE_SEPOLIA_RPC_URL` | (sourced via file) | PRESENT |
| `EXECUTION_RPC_URL` | (sourced via file) | PRESENT |

Per Phase A step 3: the operator had already added the keys to the
private file before this milestone fired, so no `OWNER_PRIVATE_KEY`
write was required by this run. The file was `chmod 600`'d
explicitly for belt-and-suspenders.

---

## 3. Derived-address verification

| Variable | Derived address | Expected | Match |
|---|---|---|---|
| `OWNER_PRIVATE_KEY` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | ✓ |
| `DEPLOYER_PRIVATE_KEY` | `0xc35F7A8A103A9A4464adfaa76B9B514093D23C27` | (same EOA as owner) | ✓ |
| `BUYER_PRIVATE_KEY` | `0x394291A05D3df2d1D8bFCBc571dAD773Ac7077cC` | `BUYER_ADDRESS` from private file | ✓ |
| `SELLER_PRIVATE_KEY` | `0xb1f1ae6CB0d154AFe9503c3B0790adeF0851FD88` | `SELLER_ADDRESS` from private file | ✓ |

Derivation done via `cast wallet address --private-key …` inside
a subshell; only the derived address (public) was emitted. Private
keys never printed.

---

## 4. Generated testnet wallets

**None.** All 4 keys were already present and valid. Phase B
wallet-generation (`cast wallet new`) was not invoked.

---

## 5. Private file state

| Property | Value |
|---|---|
| Path | `~/DEOPT/private/operator-private/sepolia.inputs.private.env` |
| Mode | `600` (verified after explicit chmod) |
| Tracked in any git repo? | NO (outside backend root; backend root is a separate git repo) |
| Modified by this milestone? | YES — `OPTION_MARGIN_ENGINE_LENS_ADDRESS` was appended after BS-2 deploy |
| File contents printed at any point? | NO |

---

## 6. Forbidden checks

| Check | Result |
|---|---|
| Private key value printed? | NO |
| RPC URL printed? | NO |
| Mainnet RPC touched? | NO |
| Production `.env` edited? | NO |
| AWS / KMS called? | NO |
| Safe tx invoked? | NO |

---

## 7. Cross-links

* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` (this run's main result)
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_PARTIAL_RESULT.md` (the prior stop)
* `docs/SEPOLIA_SETUP_FIXES_PACK_PREFLIGHT_RESULT.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`

**End of owner-key-fill result.**
