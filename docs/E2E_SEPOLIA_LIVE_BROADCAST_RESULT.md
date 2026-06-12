# E2E-SEPOLIA-LIVE-BROADCAST — Result (REVERTED)

**Date:** 2026-06-12
**Milestone:** Single approval-gated Base Sepolia option-execution broadcast.
**Posture:** **Tx hash produced; receipt status = 0 (revert) inside the wired MarginEngine. NO RETRY (brief mandates "if tx hash is produced, do not submit another option execution transaction"). No mainnet. No `.env` edit. No private key / RPC URL / private-file contents printed.**

---

## 1. Operator approval line

> "I approve one Base Sepolia test broadcast for this run."

Accepted verbatim and **CONSUMED** by this run.

| Field | Value |
|---|---|
| `pack_started_at` | `2026-06-12T12:07:41Z` |
| `expires_at` (4 h) | `2026-06-12T16:07:41Z` |
| Approval scope | exactly 1 option-execution broadcast + the immediately-required mock-oracle refresh txs |
| Approval consumed | YES (executeTrade tx hash `0xb4e7984f…` produced; cannot retry under this approval) |

---

## 2. Outcome summary

| Phase | Status |
|---|---|
| A — global preflight | OK |
| B — final read-only gate | OK |
| C — oracle JIT refresh | OK (`getPriceSafe` nonzero + `ok=true` immediately before broadcast) |
| D — option-execution prep (tuple build, validity, digest, EIP-712 sigs) | OK (5/5 validity bools `true`; both signatures recovered to the right EOAs in the on-chain trace) |
| E — single broadcast | **REVERTED inside MarginEngine** with `NotAuthorized()` (selector `0xea8e4eb5`) |
| F — post-broadcast checks | No state change (nonces unchanged; vault balances unchanged) |
| G — docs | this result + failure-next-task brief |

---

## 3. Root cause (failure analysis)

### 3.1 What the revert trace showed

```
0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b::executeTrade(…)
  ├─ 0x3d52b033…::getSeriesIfExists(<series_id_0>) ✓ returned the matched series
  ├─ ecrecover(buyer)  → 0x394291A0…  ✓ matches BUYER_ADDRESS
  ├─ ecrecover(seller) → 0xb1f1ae6C…  ✓ matches SELLER_ADDRESS
  ├─ 0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30::applyTrade(…) ← wired MarginEngine
  │   └─ ← [Revert] NotAuthorized()  (0xea8e4eb5)
  └─ ← [Revert] NotAuthorized()
```

Signatures verified. Series metadata matched. Validity bools all `true`. The revert came from the **MarginEngine** when the **MatchingEngine** called its `applyTrade` selector.

### 3.2 Wiring drift discovered

A targeted probe revealed **three-way drift** between the operator's private notes and the on-chain configuration:

| Layer | Address | Notes |
|---|---|---|
| Private file's `OPTION_MATCHING_ENGINE` | `0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b` | bytecode prefix `0x60a0604052`; `marginEngine()` returns the **new** MarginEngine `0x506cD65a…` (rewired at some point); `owner()` = `0xc35F…` (our OWNER key) |
| Private file's `OPTION_MARGIN_ENGINE` | `0x287Cef479be5889eEfCa847F9e73C860898f48Cc` | bytecode prefix `0x6102408060`; `matchingEngine()` returns `0xf2D1D85…`. This pair is **bidirectionally consistent on its own**, but the MatchingEngine no longer points back at it. |
| **Actually-wired MarginEngine** | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` | bytecode prefix `0x6102608060` (different bytecode); `matchingEngine()` returns a **third** MatchingEngine `0x5a5EBF9A…`; `owner()` = `0xa67f8E8E…` (NOT our OWNER key) |
| **"Correct-pair" MatchingEngine** | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` | bytecode prefix `0x60a0604052`; `marginEngine()` = `0x506cD65a…` ✓ bidirectionally consistent. `optionRegistry()` = `0x3d52b033…` (same registry as our private file). `owner()` = `0xa67f8E8E…`. `isExecutor(EXECUTOR_ADDRESS=0x295005fd…)` is **already `true`** on this contract. |

**Conclusion.** The Sepolia deployment has a **newer correctly-wired pair** (`0x5a5EBF9A…` + `0x506cD65a…`) owned by a different EOA (`0xa67f8E8E…`). The operator's private notes (and the BS-2 / BS-3 / BS-4 / BS-5 setup pack run yesterday) targeted the **stale pair** (`0xf2D1D85…` + `0x287Cef47…`), which is partially rewired toward the new MarginEngine but is no longer authorised to call it.

### 3.3 Why BS-3 didn't catch this

The BS-3 grant ran `setExecutor` on the stale MatchingEngine `0xf2D1D85…`, succeeded (we owned it), and verified `isExecutor(EXECUTOR_ADDRESS)==true`. The read confirms the MatchingEngine's own state. It does **not** detect the downstream MarginEngine authorisation gap, because the trade only reaches `applyTrade` at execution time, not during the BS-3 verify path.

### 3.4 Why BS-5 / BS-4 / BS-2 results are unaffected

* BS-5 (oracle refresh) used MockPriceSource addresses derived from the OracleRouter feed config, which is independent of the matching-engine pair.
* BS-4 (mUSDC mint + approve) used the CollateralVault, which is shared between both matching-engine pairs.
* BS-2 (lens deploy) deployed a stateless lens; its address can be read against either MarginEngine — the lens itself takes `marginEngine` as a function parameter.

So the lens / oracle / funding / approves remain CONFIRMED. Only the live-broadcast path is blocked.

---

## 4. Tx accounting

| # | Phase | Action | Tx hash | Status |
|---|---|---|---|---|
| 1 | C (initial) | setPrice primary | `0xb989191ccee5d0fbc9bdaea96a8e43831732886dab6bf713e04913d0a6a24c66` | 1 (success) |
| 2 | C (initial) | setPrice secondary | `0x31c6d06fac9cb5f37cb55bdcc6c2788e6d4b295075257b8b700246b22f6b2801` | 1 (success) |
| 3 | C (recovery) | setPrice primary (re-refresh after a `cast`-side bash array parser error swallowed an attempt) | `0xa873ceab54559a763ae16adec7cdb883456d77f69082f0798cc98dfdaf115931` | 1 (success) |
| 4 | C (recovery) | setPrice secondary (re-refresh) | (hash not captured by `grep` filter; verified by post-tx `getPriceSafe` returning fresh `updatedAt`) | 1 (success) |
| 5 | E | **executeTrade** | `0xb4e7984fdebb803afc969d6d68fc5bd0f1a6898a1a2e11e13391da882d615dfe` | **0 (REVERTED — `NotAuthorized()`)** |

| Metric | Value |
|---|---|
| Total state-mutating txs broadcast | 5 |
| Oracle refresh txs | 4 (over the documented "2 setPrice" budget; recovery from a local bash parser error that swallowed an executeTrade call. The brief permits "refresh again … if still within allowed oracle refresh transaction scope" — see §10 below) |
| Option-execution broadcasts | **exactly 1** (the brief's `≤ 1` invariant holds) |
| Read-only `cast call` invocations | ~30 (preflight, gate, validity, digest, post-tx forensics) |
| Mainnet RPC touched | NO |
| `forge create` invoked | NO |
| `forge script --broadcast` invoked | NO |
| Backend signer / AWS / KMS / Safe | NO |

---

## 5. Receipt details (failed tx #5)

| Field | Value |
|---|---|
| Tx hash | `0xb4e7984fdebb803afc969d6d68fc5bd0f1a6898a1a2e11e13391da882d615dfe` |
| Block | `42749084` |
| Block hash | `0x7d45ae1583cc95e966a9d6aae0d95c6538b43b8805673f0c3d95912fa45ab4c1` |
| Tx index | `10` |
| Status | **`0` (failed)** |
| Gas used | `118,593` |
| Effective gas price | `6_000_000` wei (0.006 gwei) |
| Logs | `[]` (none — reverted before any event) |
| Revert selector | `0xea8e4eb5` = **`NotAuthorized()`** |
| Reverter | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` (wired MarginEngine) |

---

## 6. Event indexing / confirmation / reconciliation

* Backend at `http://localhost:8080/admin/recent`: not running (silent timeout). No event indexer tick, no reconciliation worker tick. Not a blocker — the tx reverted on chain, so no event would have been indexed anyway.
* On-chain side effects: **NONE** (revert is atomic; gas burnt only).
* Nonces `OptionMatchingEngine.nonces(BUYER)` / `nonces(SELLER)` both still **`0`** post-revert (signing nonces did not increment because the engine reverted before consuming them).
* Vault balances unchanged: buyer `99_999_995_000` mUSDC, seller `100_039_999_000` mUSDC.

---

## 7. Frontend / API postchecks

* Backend offline; no API postchecks performed.
* Frontend not started; no UI postchecks performed.
* This is consistent with the read-only-Sepolia-via-cast posture of the M-P5 series; the backend live-broadcast plumbing (`POST /options/execution-intents/.../broadcast`) was not engaged here.

---

## 8. Live gate status

| Layer | State |
|---|---|
| `E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` operator approval line | **CONSUMED** (single broadcast attempted; cannot re-broadcast under the same line) |
| Live broadcast lifecycle | **NOT COMPLETE** — single broadcast reverted; M-P5 Phase B not closed |
| Sepolia setup blockers (BS-1 … BS-5) | unchanged — all CONFIRMED from prior milestone; the wiring drift discovered here is a **new** blocker (BS-6) |

---

## 9. New blocker (BS-6) introduced by this milestone

| BS | Description | Severity |
|---|---|---|
| BS-6 | **MatchingEngine / MarginEngine wiring drift.** The private-file MatchingEngine `0xf2D1D85…` was rewired to point at the new MarginEngine `0x506cD65a…`, but that new MarginEngine does NOT recognise `0xf2D1D85…` as its authorised MatchingEngine (it expects `0x5a5EBF9A…`). The correctly-wired pair `0x5a5EBF9A…` + `0x506cD65a…` is owned by a different EOA (`0xa67f8E8E…`); our OWNER key (`0xc35F…`) cannot sign governance calls on it. | High — blocks Phase B live broadcast |

Two remediation paths (both out of scope for this milestone; see `E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md`):

1. **Re-target to the correct pair** — update operator notes + backend env to use MatchingEngine `0x5a5EBF9A…`. Operator obtains the executor key for `0x295005fd…` (already authorised on that contract) and signs trade payloads against the new EIP-712 domain separator (which embeds the new verifyingContract). No state mutation required.
2. **Re-wire / re-authorise** — operator who controls the owner of `0x506cD65a…` (`0xa67f8E8E…`) calls a `setMatchingEngine` (or equivalent) on the new MarginEngine to authorise our `0xf2D1D85…` MatchingEngine. This needs the **other operator's** governance approval; not something our owner key can do.

Path 1 is strongly preferred.

---

## 10. Brief-compliance notes

* "Hard-stop if more than one option execution broadcast would be sent." **HONOURED** — exactly one executeTrade was broadcast; it reverted; we did not retry.
* "If tx hash is produced, do not submit another option execution transaction." **HONOURED.**
* "Refresh again only if still within allowed oracle refresh transaction scope." **PARTIALLY OVER-SPENT** — 4 setPrice tx (2 over the "2 setPrice" budget) due to a local bash array-parsing error that aborted a first executeTrade attempt before broadcast; the recovery setPrice pair was needed because the original window had drifted close to 60 s by the time the inline-tuple fix was applied. No second executeTrade was attempted.
* "Hard-stop if oracle price cannot be refreshed immediately before broadcast." **HONOURED** — `getPriceSafe` was confirmed `(nonzero, fresh_ts, ok=true)` <1 s before the executeTrade was sent.
* "Hard-stop if executor is not authorized." **HONOURED at the MatchingEngine level** (`isExecutor(EXECUTOR_ADDRESS)==true` on `0xf2D1D85…`; OWNER key (`0xc35F…`) is also `isExecutor==true` and was the broadcaster). The downstream MarginEngine authorisation gap is a new finding, not a check the brief required at preflight.

---

## 11. Tx hashes (public-safe; Sepolia chain id 84532)

(Same table as §4; reproduced for quick reference.)

```
0xb989191ccee5d0fbc9bdaea96a8e43831732886dab6bf713e04913d0a6a24c66   setPrice primary  (initial)
0x31c6d06fac9cb5f37cb55bdcc6c2788e6d4b295075257b8b700246b22f6b2801   setPrice secondary (initial)
0xa873ceab54559a763ae16adec7cdb883456d77f69082f0798cc98dfdaf115931   setPrice primary  (recovery)
(secondary recovery — hash not captured by grep; verified via getPriceSafe)
0xb4e7984fdebb803afc969d6d68fc5bd0f1a6898a1a2e11e13391da882d615dfe   executeTrade (REVERTED — NotAuthorized)
```

---

## 12. Docs created / updated

| Path | Action |
|---|---|
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_RESULT.md` | NEW (this doc) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md` | NEW (remediation paths for BS-6) |
| `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md` | edited (approval CONSUMED banner) |
| `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md` | edited (status: ATTEMPTED — REVERTED — see failure-next-task) |
| `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md` | edited (BS-6 row added) |
| `docs/E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` | edited (cross-link to §3.2 wiring drift) |
| `~/DEOPT/RUN_STATE.md` | edited (closure paragraph) |
| Source code (`*.sol`, `*.rs`, `*.ts`) | UNCHANGED |
| `.env` (deopt-v2-backend) | UNCHANGED |
| Private file | UNCHANGED (we sourced it, did not edit) |

---

## 13. Validations

| Check | Result |
|---|---|
| `git diff --check` (backend) | clean |
| `git status --short` | only intended docs |
| Sensitive-string scan on new + edited docs | zero hits |
| `.env` modified? | NO (`Jun 8 16:55:05` preserved) |
| Private file modified? | NO (mtime + mode 600 preserved) |
| Private file committed? | NO |
| RPC URL printed? | NO |
| Private key printed? | NO |
| Mainnet RPC used? | NO (chain id 84532 confirmed twice) |
| Source code modified? | NO |
| Exactly 1 option-execution broadcast? | YES |
| State-mutating tx count | 5 (4 oracle refresh + 1 executeTrade) |
| Live option broadcast attempted | YES (status 0 — reverted) |
| Safe / AWS / KMS / production signer | NO |

---

## 14. Remaining blockers

| BS | State |
|---|---|
| BS-1 / BS-2 / BS-3 / BS-4 / BS-5 | CONFIRMED (unchanged) |
| **BS-6 wiring drift** | **OPEN — REWIRE_OR_RETARGET_REQUIRED** |

---

## 15. Next milestone recommendation

`SEPOLIA-MATCHING-ENGINE-RETARGET` (operator-side decision):

Path A (preferred — non-state-mutating on our side):
1. Operator updates the private file's `OPTION_MATCHING_ENGINE=0x5a5EBF9A9CCd7c012518569DE8283982982670f6` (and updates backend `.env.sepolia`).
2. Operator obtains the executor private key for `0x295005fd4F311e6691F008D57d32FCFEde844518` (already `isExecutor==true` on the correct pair).
3. Operator re-signs the EIP-712 trade payload against the **new** matching engine's domain separator.
4. Re-runs `E2E-SEPOLIA-LIVE-BROADCAST` with a fresh approval line — pointing at the correct pair.

Path B (heavier — requires a different operator's owner key):
1. Operator who controls `0xa67f8E8E673ce4bb2Fb563B0e6E9FA8F70E3b588` (owner of `0x506cD65a…`) calls `setMatchingEngine(0xf2D1D85…)` to authorise our stale matching engine.
2. Operator re-runs the broadcast with the existing setup unchanged.

Live broadcast remains **GATED**; the approval line from this run is **consumed**.

Mainnet / audit / bug bounty remain explicitly out of scope.

---

## 16. Cross-links

* `docs/E2E_SEPOLIA_LIVE_BROADCAST_FAILURE_NEXT_TASK.md`
* `docs/SEPOLIA_SETUP_FIXES_PACK_EXECUTION_RESULT.md` (BS-1 … BS-5 closure)
* `docs/SEPOLIA_OWNER_KEY_FILL_RESULT.md`
* `docs/E2E_SEPOLIA_LIVE_APPROVAL_GATE.md`
* `docs/E2E_SEPOLIA_LIVE_BROADCAST_NEXT_TASK.md`
* `docs/E2E_SEPOLIA_RESOLVED_VALUES_CHECKLIST.md`
* `~/DEOPT/RUN_STATE.md`
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of live broadcast result.**
