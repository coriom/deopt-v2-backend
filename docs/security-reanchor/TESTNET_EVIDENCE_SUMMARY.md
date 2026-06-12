# DeOpt V2 — Testnet Evidence Summary

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

What we can demonstrate from the Base Sepolia testnet today. **This is not an audit. This is not mainnet safety. Successful testnet evidence does not prove protocol security.** Read accordingly.

---

## 1. Canonical Sepolia trade

### 1.1 Tx-level

| Field | Value |
|---|---|
| tx hash | `0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a` |
| block | `42750521` |
| chain id | `84532` (Base Sepolia) |
| status | `1` (success) |
| gas used | `683_044` |
| logs emitted (raw) | 23 |
| logs normalised by indexer | 19 |
| timestamp | 2026-06-12 (per `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`) |
| executor (broadcaster) | operator-controlled testnet EOA (key not printed; out of scope here) |
| matching engine target | `0x5a5EBF9A9CCd7c012518569DE8283982982670f6` (canonical) |
| margin engine target | `0x506cD65a63C53c66ab572B9f9dd819B7BfE00D30` (canonical) |
| basescan | `https://sepolia.basescan.org/tx/0x748c94843cb4cbe31f56c84ceedc7e000a05dac567fa3fe7a1415a0de59b637a` |

### 1.2 What this proves

* The deployed `OptionMatchingEngine` accepts a valid two-sided EIP-712 signature pair.
* The `MarginEngine` accepts the matching engine as an authorised caller post-retarget (bidirectional wiring).
* The `OracleRouter` returned `ok=true` at broadcast time (otherwise the trade would have reverted).
* The `FeesManagerV2` emitted fee events for both sides.
* The end-to-end pipeline (frontend signing → backend intent → executor broadcast → on-chain settle → indexer event) works for at least one trade.

### 1.3 What this does NOT prove

* Protocol-level safety under adversarial conditions.
* Vault accounting holds under arbitrary sequences (would need invariant tests).
* Signature model is replay-safe across all (signer, intent) shapes (would need fuzz / formal review).
* Fee accounting holds under partial fills, cancellations, or settlements (would need broader test coverage).
* Mainnet readiness (it does NOT prove this; mainnet is deliberately out of scope).
* Multi-user concurrency safety (single trade observed).
* Oracle adversarial resistance (testnet uses `MockPriceSource`).

---

## 2. Backend reconciliation

### 2.1 Indexer + reconciliation worker

* `OPTION_EVENT_INDEXER_ENABLED=true` worker reads chain events and normalises into Postgres rows.
* `OPTION_RECONCILIATION_WORKER_ENABLED=true` worker reconciles `option_execution_intents → option_execution_transactions → option_execution_events → option_execution_reconciliations`.
* Post-broadcast catch-up confirmed in `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`.

### 2.2 Indexed event counts (canonical trade)

* 19 normalized events captured by the indexer for the canonical trade.
* Breakdown (per the reconciliation result doc): `OptionTradeExecuted` + accrual events + transfer events + fee-event split. Detailed mapping in `SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md`.

### 2.3 Restart safety

* Backend can be restarted with all broadcast gates disabled:
  * `EXECUTION_ENABLED=false`
  * `EXECUTOR_REAL_BROADCAST_ENABLED=false`
  * `OPTION_EXECUTION_BROADCAST_ENABLED=false`
  * Indexer + reconciliation workers still operating.
* No chain transaction is sent on cold startup. Verified in the reconciliation-fix result doc.

### 2.4 What this proves

* The backend can rejoin chain state without re-broadcasting the trade.
* `/trading/health` returns `chain_id=84532` and `rpc_reachable=true` against the testnet RPC.
* Lifecycle endpoint converges to `reconciliation.status: ok` after the worker tick.

### 2.5 What this does NOT prove

* The reconciliation is robust under chain reorgs deeper than the configured confirmation depth.
* The indexer recovers from intentional state poisoning (e.g. a maliciously crafted log sequence).
* Backend test coverage of the reconciliation path is complete (test inventory still owed — see `PRE_AUDIT_ACTION_PLAN.md`).

---

## 3. Nonce / balance / position / fee accounting

### 3.1 Nonce

* `nonces(buyer)` and `nonces(seller)` both incremented by 1 after the canonical trade. Verifiable via `cast call`.
* No double-execution observed (would have reverted with `NonceConsumed()` or equivalent).

### 3.2 Balance / vault

* Buyer's mUSDC balance in `CollateralVault` decreased by premium + fee.
* Seller's mUSDC balance in `CollateralVault` increased by premium minus fee.
* Net vault balance delta matches the fee accrual to `ProtocolFeeVault`.

### 3.3 Position

* Buyer's `long` position quantity increased by trade size.
* Seller's `short` position quantity increased by trade size.
* Sum of buyer-long + seller-short for the series is zero (zero-sum invariant holds for this single trade).

### 3.4 Fee

* `FeesManagerV2` emitted both a `BuyerFee` and a `SellerFee` event.
* Sum of `BuyerFee.amount + SellerFee.amount` matches the net protocol fee credit.
* `ProtocolFeeVault.balanceOf(mUSDC)` increased by the same amount.

### 3.5 What this proves

* The accounting holds **for this one trade** under the observed parameters.

### 3.6 What this does NOT prove

* Accounting holds for **any** trade. (Need invariant tests.)
* Accounting holds across multiple concurrent trades.
* Accounting holds across cancellation / exercise / settle paths.

---

## 4. Frontend build + test status

### 4.1 Static checks

| Command | Result | When |
|---|---|---|
| `npm run typecheck` (`tsc --noEmit`) | clean | 2026-06-12 |
| `npm run lint` (`eslint`) | clean | 2026-06-12 |
| `npm run build` (`next build`) | green, 9 routes prerendered | 2026-06-12 |

### 4.2 Playwright

* `npx playwright test --list` — 30 tests in 12 files.
* Targeted execution not run in this sandbox (WSL2 missing `libnspr4.so`; CI / Linux unaffected). CI execution log will provide actual run evidence; deferred to the `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` milestone.

### 4.3 What this proves

* The codebase compiles + passes static checks at the freeze moment.
* The spec graph is structurally sound.

### 4.4 What this does NOT prove

* The specs actually pass when executed (catalog-only).
* Coverage is complete.
* The bundle is free of accidental secret inclusion (separately covered by the `no-admin-bearer` spec + the public-beta-footer DOM scan in the same suite).

---

## 5. Public beta docs status

### 5.1 Inventory

* 15 docs under `deopt-v2-backend/docs/public-beta/`, ~2,481 lines total.
* All canonical contract addresses present + verifiable.
* Stale ME `0xf2D1D85…` explicitly flagged "DO NOT USE" in every contract-address doc.
* Canonical first-trade tx referenced where useful.
* No bearer tokens / RPC URLs with keys / DATABASE_URLs anywhere in the public-beta directory.

### 5.2 Tester safety

* `BUG_REPORT_TEMPLATE.md §1` enforces no private values in reports.
* `FEEDBACK_TRIAGE_WORKFLOW.md §5` enforces redact-and-rotate on accidental leaks.
* `COMMUNITY_ONBOARDING.md §4` enforces "never share your private key or seed phrase".
* Every announcement draft has an honesty checklist preventing positive-claim drift.

### 5.3 What this proves

* The docs pack is internally consistent + public-safe.

### 5.4 What this does NOT prove

* Testers will read the disclaimers. (Banners + footers on every trading route help.)

---

## 6. Community feedback loop status

### 6.1 Infrastructure

* 6 community-feedback docs (templates + triage workflow + onboarding + launch checklist + announcement drafts + operator URLs-fill).
* Frontend public-beta link config wired (placeholders, ready for substitution).
* Sign-failure modal "Report this issue" CTA wired with placeholder degradation.

### 6.2 What this proves

* The infrastructure exists to receive feedback.

### 6.3 What this does NOT prove

* Real channels are live. (Operator must complete `OPERATOR_PUBLIC_BETA_URLS_FILL.md` before announcement.)
* Volume of feedback is meaningful (no public announcement has been sent).

---

## 7. Closing reminder

Testnet evidence is **necessary but not sufficient** for either audit-readiness or mainnet-readiness:

* Necessary because an auditor will ask for proof the protocol functions end-to-end. We have one canonical trade.
* Not sufficient because security comes from invariant proofs, fuzz testing, code review, formal analysis, monitoring, and incident response — none of which a single passing trade can substitute for.

Do **not** cite this evidence as audit readiness. Do **not** cite it as mainnet safety. Cite it only as: "the testnet beta lifecycle works for at least one trade and the backend can recover the state without re-broadcasting."

---

**End of testnet evidence summary.**
