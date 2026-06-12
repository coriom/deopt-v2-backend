# SEPOLIA-MATCHING-ENGINE-RETARGET — Result (Path A)

**Date:** 2026-06-12
**Milestone:** No-broadcast remediation of BS-6 (wiring drift) discovered in `E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md`.
**Posture:** **No `cast send`. No `forge create`. No `forge script --broadcast`. No state mutation on chain. No mainnet. No `.env` edit. No private key / RPC URL / private-file contents printed.**

---

## 1. Outcome summary

**BS-6 CLOSED (RETARGET_CONFIRMED).** The operator's private notes now point at the correctly-wired MatchingEngine + MarginEngine pair on Base Sepolia. EIP-712 dry-run against the new ME confirms a clean broadcast path; only blocker remaining for the actual retry is operator supply of `EXECUTOR_PRIVATE_KEY` (the new ME requires the executor EOA `0x295005fd…` as the broadcaster — our `OWNER_PRIVATE_KEY` is NOT `isExecutor==true` on it).

---

## 2. Workspace / window

| | |
|---|---|
| Pack started | `2026-06-12T12:30:07Z` |
| Mode | read-only Sepolia; chain id 84532 enforced |
| Approval line | none required — this milestone does not broadcast |

---

## 3. Private input handling

| Variable | Status |
|---|---|
| Private file at `~/DEOPT/private/operator-private/sepolia.inputs.private.env` | PRESENT, mode `600`, outside any git repo |
| `BASE_SEPOLIA_RPC_URL` / `EXECUTION_RPC_URL` | PRESENT (never printed) |
| All 12 contract / role addresses | PRESENT |
| `OWNER_PRIVATE_KEY` / `DEPLOYER_PRIVATE_KEY` / `BUYER_PRIVATE_KEY` / `SELLER_PRIVATE_KEY` | PRESENT (presence-only audit) |
| `EXECUTOR_PRIVATE_KEY` | MISSING (required for the retry broadcast — see §10) |
| Lines edited | exactly 2 (`OPTION_MATCHING_ENGINE`, `OPTION_MARGIN_ENGINE`) via `sed -i -E` with `export ` prefix preserved |
| `chmod 600` after edit | YES |
| File contents printed at any point | NO |
| Tracked in git | NO |

A historical-traceability comment block was appended (commented-out only — no live values):

```
# Retarget 2026-06-12 — old stale wiring (kept for historical traceability; NOT used):
# LEGACY_OPTION_MATCHING_ENGINE_PRE_RETARGET_2026_06_12=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
# LEGACY_OPTION_MARGIN_ENGINE_PRE_RETARGET_2026_06_12=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
```

---

## 4. Global read-only preflight

| Check | Result |
|---|---|
| `cast chain-id` | **84532** (Base Sepolia) |
| Mainnet hard-stop triggered? | NO |
| CORRECT_ME (`0x5a5EBF9A…`) bytecode | PRESENT (20,279 hex chars) |
| WIRED_MARGIN (`0x506cD65a…`) bytecode | PRESENT (48,929 hex chars) |
| STALE_ME (`0xf2D1D85…`) bytecode | PRESENT (15,089 hex chars) — kept for documentation |
| STALE_MARGIN (`0x287Cef47…`) bytecode | PRESENT (48,805 hex chars) — kept for documentation |

---

## 5. Correct-wiring verification (read-only)

### 5.1 New MatchingEngine `0x5a5EBF9A9CCd7c012518569DE8283982982670f6`

| Selector | Result |
|---|---|
| `marginEngine()` | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` ✓ |
| `owner()` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (different operator) |
| `optionRegistry()` | `0x3d52b033Fab00ed6104DD3bc0a715F8648344ecA` (same registry as private file) |
| `guardian()` | `0xA6B9Bb5c7B26B33cfD28C6F5A79B3c527fDdcD46` |
| `paused()` | `false` |
| `isExecutor(EXECUTOR_ADDRESS = 0x295005fd…)` | **`true`** (no governance call needed) |
| `isExecutor(0xc35F…)` (our OWNER key) | `false` — our owner cannot directly broadcast `executeTrade` on this ME |

### 5.2 New MarginEngine `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`

| Selector | Result |
|---|---|
| `matchingEngine()` | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` ✓ |
| `owner()` | `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (same owner as the ME) |

### 5.3 Bidirectional assertion

| Assertion | Result |
|---|---|
| `CORRECT_ME.marginEngine() == WIRED_MARGIN` | **YES** |
| `WIRED_MARGIN.matchingEngine() == CORRECT_ME` | **YES** |
| Wiring | **BIDIRECTIONAL_OK** |

### 5.4 EIP-712 domain (new ME)

```
name              : "DeOptV2-OptionMatchingEngine"
version           : "1"
chainId           : 84532
verifyingContract : 0x5a5EBF9A9CCd7c012518569DE8283982982670f6
salt              : 0x0…
extensions        : []
domainSeparator   : 0x68d1704576b276dd7c71b56f36643fdc3fc7c4cee67830362e3488f207cfbfb8
```

This differs from the stale-ME domain (verifyingContract `0xf2D1D85…`), so old signatures from the reverted broadcast are invalid here. New signatures were re-derived in §7.

---

## 6. Private retarget

| Field | Before | After |
|---|---|---|
| `OPTION_MATCHING_ENGINE` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` (stale) | **`0x5a5EBF9A9CCd7c012518569DE8283982982670f6`** |
| `OPTION_MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` (stale) | **`0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30`** |
| All other variables | (unchanged) | (unchanged) |
| File mode | `600` | `600` |
| Tracked in any git repo | NO | NO |
| Lines disturbed by `sed -i` | exactly 2 | (verified via sourced-subshell readback) |

The legacy values are preserved as commented-out lines (per §3) so the operator retains a paper trail without breaking the active config.

---

## 7. EIP-712 dry-run on the new ME (no broadcast)

Trade tuple targeting **series #0** (call on `0x4DeE…`, strike `300_000_000_000` 1e8, contract size 1e8, expiry `1893456000`):

| Field | Value |
|---|---|
| intentId | `0x…` (random per-run; never reused) |
| buyer / seller | private-file public addresses |
| optionId | series #0 (decimal `24145…0183`) |
| underlying / settlement | `0x4DeE…` / `0x6eAe…` |
| expiry | `1893456000` (uint64) |
| strike1e8 | `300_000_000_000` (uint64) |
| isCall | `true` |
| contractSize1e8 | `100_000_000` (uint128) |
| quantity | `1` |
| premiumPerContract | `1_000_000` (= 1 mUSDC native) |
| buyerIsMaker | `false` |
| buyerNonce / sellerNonce | **`2` / `2`** (matches `nonces(BUYER)` / `nonces(SELLER)` on the NEW ME; the reverted broadcast on the stale ME did not advance these — see §9) |
| deadline | `NOW + 7200` (2 hours) |

### 7.1 `previewTradeValidity(t)` (view)

| Flag | Result |
|---|---|
| structurallyValid | **true** |
| deadlineValid | **true** |
| buyerNonceValid | **true** (with nonce=2) |
| sellerNonceValid | **true** (with nonce=2) |
| seriesMetadataValid | **true** |
| digest returned | `0xd3e40ef9ebd665a459b3bf89e75ce84562d11d9ae33235f867606adec24cbfa1` |

### 7.2 `previewTradeDigest(t)` (view)

Returns `0xd3e40ef9ebd665a459b3bf89e75ce84562d11d9ae33235f867606adec24cbfa1`.

### 7.3 `hashTrade(t)` (view) — canonical cross-check

Returns the same value. `hashTrade == previewTradeDigest` ✓.

### 7.4 Signature recovery (off-chain, then verified via ecrecover precompile)

| Step | Result |
|---|---|
| `cast wallet sign --no-hash` with `BUYER_PRIVATE_KEY` | 132-char signature (r,s,v) |
| `cast wallet sign --no-hash` with `SELLER_PRIVATE_KEY` | 132-char signature (r,s,v) |
| `ecrecover` precompile (`0x…01`) on buyer sig | **recovers to `0x394291a0…cC` = `BUYER_ADDRESS`** ✓ |
| `ecrecover` precompile on seller sig | **recovers to `0xb1f1ae6c…88` = `SELLER_ADDRESS`** ✓ |

Note: `cast wallet verify` applies an EIP-191 prefix by default and reports "did not sign". The correct verification path for EIP-712-style raw-digest signing is `ecrecover(digest, v, r, s)` directly (the same path `executeTrade` will run on chain). We verified via the `0x…01` precompile.

---

## 8. Full read-only gate (post-retarget)

| BS | Check | Result |
|---|---|---|
| BS-1 | (closed by inheritance discovery) | CLOSED |
| BS-2 | lens bytecode at `0x496A57CF…` | PRESENT |
| BS-3 | `isExecutor(EXECUTOR_ADDRESS)` on new ME | **`true`** (no governance call needed) |
| BS-4 | buyer/seller EOA mUSDC | `100_000_000_000` each |
| BS-4 | buyer/seller allowance → vault | `100_000_000_000` each |
| BS-4 | buyer/seller vault deposits | `99_999_995_000` / `100_039_999_000` |
| BS-5 | hasActiveFeed(series #0) | `true` |
| BS-5 | getPriceSafe(series #0) | **stale (`(0, 0, false)`) — needs JIT refresh before retry broadcast** |
| BS-6 | wiring bidirectional | **OK** (closed) |

All gate items OK; BS-5 freshness is a per-broadcast workflow constraint, not a remediation gap.

---

## 9. Nonce accounting (cross-ME)

| ME | `nonces(BUYER)` | `nonces(SELLER)` |
|---|---|---|
| Stale ME `0xf2D1D85…` (the reverted broadcast hit this) | `0` | `0` |
| **New ME `0x5a5EBF9A…`** (active target) | **`2`** | **`2`** |

The reverted live broadcast did **not** consume any nonce on the new ME (because it targeted the stale ME and the engine reverted before consuming the stale-ME nonce). The new-ME nonces of `2/2` reflect prior trades the operator (or a prior smoke) had run against the correct pair before this entire arc began.

The retry trade tuple MUST use `buyerNonce=2`, `sellerNonce=2`.

---

## 10. BS-6 status

**OPEN → CLOSED (RETARGET_CONFIRMED).** Path A as described in `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` §3 fully applied with no on-chain mutations:

* Private notes retargeted (§6).
* New ME / MarginEngine wiring confirmed bidirectional (§5).
* `isExecutor(EXECUTOR_ADDRESS)` on the new ME already `true` (§5.1) — BS-3 grant on the stale ME is now irrelevant but harmless.
* EIP-712 dry-run (§7) confirms 5/5 validity bools, canonical digest, and correct signature recovery.

The single remaining gap for the **retry broadcast** itself is operator supply of `EXECUTOR_PRIVATE_KEY` — without it, no key in the private file is `isExecutor==true` on the new ME, so the executeTrade call cannot be dispatched. The retry brief at `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md` captures this precondition.

---

## 11. Docs created / updated

| Path | Action |
|---|---|
| `docs/SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md` | NEW (this doc) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md` | NEW (retry brief, fresh approval line required) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` | edited (BS-6 closed banner — Path A success) |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (BS-6 row → CLOSED) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | edited (post-retarget wiring asserts) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (gate → READY FOR FRESH OPERATOR APPROVAL) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | edited (preconditions updated — new ME target, executor-key requirement) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |
| Private file | retargeted (2 lines + commented-out legacy traceability) |
| Source code (`*.sol`, `*.rs`, `*.ts`, `*.tsx`) | UNCHANGED |
| `.env` (deopt-v2-backend) | UNCHANGED |

---

## 12. Files changed

**Source code: ZERO.** Only docs + RUN_STATE + private file (retarget).

---

## 13. Validations

| Check | Result |
|---|---|
| `git diff --check` (backend) | clean |
| `git status --short` | only intended docs |
| Sensitive-string scan on new + edited docs | zero hits |
| `.env` modified? | NO (`Jun 8 16:55:05` preserved) |
| Private file modified? | YES — exactly 2 lines (`OPTION_MATCHING_ENGINE`, `OPTION_MARGIN_ENGINE`) + 1 commented-out legacy block |
| Private file mode after edit | `600` |
| Private file committed? | NO (outside any git repo) |
| RPC URL printed? | NO |
| Private key printed? | NO |
| Mainnet RPC used? | NO (chain id 84532 confirmed) |
| State-mutating command run? | NO (`cast send` / `forge create` / `forge script --broadcast` count = 0) |
| Source code modified? | NO |
| Broadcast attempted? | NO |
| Safe / AWS / KMS / production signer | NO |

---

## 14. Remaining blockers

| BS | State |
|---|---|
| BS-1 / BS-2 / BS-3 / BS-4 / BS-5 | CONFIRMED (unchanged) |
| **BS-6 wiring drift** | **CLOSED (RETARGET_CONFIRMED)** |

The live broadcast retry depends on a NEW operator action that is NOT itself a blocker on the protocol — supply of `EXECUTOR_PRIVATE_KEY` for the EOA `0x295005fd4F311e6691F008D57d32FCFEde844518`. This is captured as a hard precondition in `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md`, not as a new BS row.

---

## 15. Next milestone recommendation

`E2E-SEPOLIA-LIVE-BROADCAST-RETRY` — brief at `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md`. Requires:

1. Operator supplies `EXECUTOR_PRIVATE_KEY` for `0x295005fd…` via one of the two safe paths (append to private file OR `read -rs` + `export`).
2. Operator types a fresh approval line **verbatim**:
   > "I approve one Base Sepolia test broadcast retry for this run."
3. Harness re-runs preflight + JIT oracle refresh (2 setPrice) + executeTrade against the new ME with `buyerNonce=2`, `sellerNonce=2`, fresh deadline, re-derived digest, fresh signatures.

Live broadcast gate is now **READY FOR FRESH OPERATOR APPROVAL** but does NOT authorise the retry until that fresh approval line is supplied. Mainnet / audit / bug bounty remain explicitly out of scope.

---

## 16. Cross-links

* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RETRY_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of retarget result.**
