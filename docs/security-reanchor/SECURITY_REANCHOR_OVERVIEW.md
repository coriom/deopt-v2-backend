# DeOpt V2 — Security Re-Anchor Overview

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

This document is the cross-repo security review matrix. Each row is a component or invariant; each column is the part of the picture an external reviewer (or internal review board) needs.

* **Status** — what we have built / committed.
* **Security assumption** — what the system relies on being true.
* **Evidence** — where the assumption is anchored (doc, test, code).
* **Gap** — what is missing or weak right now.
* **Severity** — informational (I), low (L), medium (M), high (H), critical (C).
* **Before-audit action** — what must change before this row could plausibly be presented to an external auditor.
* **Before-mainnet action** — what must change before this row could plausibly support real-funds traffic.

Severity ratings are **at the testnet-beta moment**. They are not audit findings; they are operator self-assessments.

---

## 1. Contract roles + ownership

| Field | Value |
|---|---|
| Status | Contracts deployed to Base Sepolia with operator-controlled EOA owners. `OptionMatchingEngine 0x5a5EBF9A…` ↔ `MarginEngine 0x506cD65a…` bidirectional authorization in place. |
| Security assumption | Owner can pause / rotate / reconfigure. Owner key is held by the operator and not shared. Owner is a single EOA at testnet — NOT a Safe multisig. |
| Evidence | `deopt-v2-sol/abis/freeze-v2-product-rc1/`, `deopt-v2-sol/docs/SOL_PRODUCT_SCOPE_FREEZE_RESULT.md`, `MAINNET_CUSTODY_POLICY.md §R-1..R-9` (mainnet target model). |
| Gap | Owner is a single EOA. No multisig. No timelock. No emergency-pause runbook published in the public-beta pack. |
| Severity | M (testnet); H if mainnet were attempted. |
| Before-audit action | Document the testnet owner model + the **target** mainnet ownership model (Safe multisig + timelock) in the audit handoff index. |
| Before-mainnet action | Migrate to Safe multisig per `MAINNET_CUSTODY_POLICY.md §R-1` GOVERNANCE_MULTISIG (≥3-of-5). Timelock. Pause runbook. Ownership transfer dry-run on a fork. |

## 2. Executor authorization model

| Field | Value |
|---|---|
| Status | Executor EOA can call `OptionMatchingEngine.executeTrade(...)` with both signatures + trade tuple. Executor is rate-limited; matching engine verifies signatures on chain. |
| Security assumption | Executor cannot forge signatures (they're verified on chain); executor can only broadcast trades both sides already signed. |
| Evidence | `BACKEND_EXECUTOR_CUSTODY.md`, `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`, frozen ABI selectors for `executeTrade`. |
| Gap | Executor key is a plain testnet EOA (not KMS / not HSM). No alerting on executor key compromise. |
| Severity | M (testnet — no real funds); H if mainnet were attempted. |
| Before-audit action | Document the executor key handling (where it lives, how it's loaded, what protects it). Confirm in writing that the testnet executor key is testnet-only. |
| Before-mainnet action | KMS-backed executor signer per `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`. Per-tx alerting per `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`. Key rotation runbook. Compromise IR runbook. |

## 3. Option trade signature model (EIP-712)

| Field | Value |
|---|---|
| Status | Two-sided EIP-712 signatures (buyer + seller) over a typed `OptionTrade` payload. Domain separator anchored on chain id `84532` + verifying contract `0x5a5EBF9A…`. Nonces per signer. |
| Security assumption | A trade cannot be replayed (nonce monotonically non-decreasing per signer per side). A signature for one (under, settle, side, qty, strike, expiry) tuple cannot be reused for another. Domain separator binds to chain id + verifying contract. |
| Evidence | `deopt-v2-frontend/src/lib/eip712.ts`, frozen ABI, USER_TESTING_GUIDE.md (sample signature payload). |
| Gap | No published invariant doc proving the signature model is replay-safe. No fuzz tests cited. No formal verification. |
| Severity | M. |
| Before-audit action | Write an `INVARIANTS.md` doc (sketched in `PRE_AUDIT_ACTION_PLAN.md`) enumerating: nonce monotonicity, domain-separator binding, deadline enforcement, signature-uniqueness across (intent_id, signer) pairs. Cite tests. |
| Before-mainnet action | External audit must affirm the signature model. Mainnet domain separator must use mainnet chain id (would require redeploy + re-sign of all open intents — which there will be none at deploy time). |

## 4. Nonce model

| Field | Value |
|---|---|
| Status | Per-signer nonce stored on the matching engine. Incremented atomically on successful `executeTrade`. |
| Security assumption | `nonces(addr)` is monotonically non-decreasing per signer. A signed tuple with `nonce = N` is accepted iff `nonces(addr) <= N` and is rejected after `nonces(addr) > N` advances. |
| Evidence | Frozen `OptionMatchingEngine` ABI; relevant view function `nonces(address)` in `selectors.txt`. |
| Gap | No public test reproduction of "double-execution attempt is rejected". |
| Severity | M. |
| Before-audit action | Cite the nonce invariant in `INVARIANTS.md`. Provide at least one negative test (the same signature submitted twice returns `NonceConsumed()` or equivalent). |
| Before-mainnet action | External audit must explicitly affirm nonce model and review the `executeTrade` revert-on-stale-nonce path. |

## 5. Oracle assumptions

| Field | Value |
|---|---|
| Status | `OracleRouter` with `maxDelay = 60 s`. Backed by `MockPriceSource` on Sepolia (testnet beta only). `getPriceSafe(under, settle)` returns `(price_1e8, age_ms, ok)`; trades require `ok == true`. |
| Security assumption | Stale price → trade reverts. Oracle cannot be manipulated to inject a stale-but-fresh-looking price. |
| Evidence | Frozen ABI, public-beta docs `KNOWN_LIMITATIONS_AND_RISKS.md`, executor refusal logic in backend. |
| Gap | Testnet uses a mock oracle (operator-controlled). No production-oracle integration yet (Chainlink / Pyth / API3 / RedStone selection pending). Operator can technically push any price during testnet — explicitly disclosed. |
| Severity | M on testnet (explicit + disclosed), C on mainnet. |
| Before-audit action | Document the **target** mainnet oracle design (which provider, which feeds, which freshness invariants, which fallback). Audit can review the mock-oracle path as the existing surface plus the design intent. |
| Before-mainnet action | Real oracle integration. Sanity bounds. Median-of-N or similar. Independent monitoring. |

## 6. Collateral / vault accounting

| Field | Value |
|---|---|
| Status | `CollateralVault` holds ERC-20 mUSDC. Deposits / withdrawals are allowance-gated. View functions on `CollateralVaultViews` expose balances + with-yield projection. |
| Security assumption | Vault balance equals sum of per-account deposits minus net withdrawals minus realised losses. No path lets a user withdraw more than `free_collateral`. Strategy / yield paths cannot drain principal. |
| Evidence | Frozen ABI, `CollateralVaultViews` selectors. |
| Gap | No external accounting reconciliation publicly published. No public invariant test asserting `sum(deposits) - sum(withdrawals) == vault.balanceOf(token)`. |
| Severity | H. |
| Before-audit action | Publish an accounting-invariant doc + Foundry invariant tests (or commit to writing them) showing the vault's bookkeeping reconciles under arbitrary deposit / withdraw / settle sequences. |
| Before-mainnet action | External audit must affirm vault accounting. Production monitoring must alert on any `vault.balanceOf` < `sum(deposits)` divergence. |

## 7. Fee accounting

| Field | Value |
|---|---|
| Status | `FeesManagerV2` emits per-trade fee events; buyer + seller fees split per `ppm_signed`. Protocol fees accrue to `ProtocolFeeVault`. |
| Security assumption | `buyer_side_fee + seller_side_fee == net_fee_to_protocol`. No fee path can be triggered without a corresponding trade. `ProtocolFeeVault.sweep` is owner-gated. |
| Evidence | Frozen ABI, `BACKEND_FEES_MANAGER_V2_ABI_AND_FEE_SPLIT_WIRING_RESULT.md`. |
| Gap | Same as §6 — no public invariant test summary. |
| Severity | H. |
| Before-audit action | Add fee-invariant rows to the accounting-invariant doc. |
| Before-mainnet action | External audit. Operator runbook for fee sweep. Treasury custody policy. |

## 8. Margin engine authorization

| Field | Value |
|---|---|
| Status | `MarginEngine` authorises `OptionMatchingEngine` (and only it) to mutate margin state. Bidirectional wiring asserted post-M-P5 retarget (canonical pair `0x5a5EBF9A… ↔ 0x506cD65a…`). |
| Security assumption | Only the authorised matching engine can call into MarginEngine. No upgrade path bypasses the authorization. |
| Evidence | Frozen ABI; `SEPOLIA_MATCHING_ENGINE_RETARGET_RESULT.md`. Legacy stale ME `0xf2D1D85…` is NOT authorised. |
| Gap | The bidirectional wiring must be re-verified on every deployment. No automated regression. |
| Severity | M. |
| Before-audit action | Cite the wiring assertion in `INVARIANTS.md`. Provide a `cast call` snippet to verify wiring on any deployment. |
| Before-mainnet action | Deployment script must verify wiring as a hard gate. Monitoring must alert on unauthorised callers. |

## 9. Event indexing assumptions

| Field | Value |
|---|---|
| Status | `OPTION_EVENT_INDEXER_ENABLED=true` indexer worker reconciles events into Postgres tables (`option_execution_intents`, `option_execution_transactions`, `option_execution_events`, `option_execution_reconciliations`, `option_event_indexer_state`). |
| Security assumption | Indexer is eventually-consistent. Reorgs are rare on Base; indexer treats `block - N` as final after a small confirmation window. Lifecycle endpoint may briefly show `reconciliation.status: missing_events` and resolve. |
| Evidence | `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`, `SEPOLIA_POST_BROADCAST_BACKEND_RECONCILIATION_RESULT.md`. |
| Gap | No documented behaviour on a deep reorg (> confirmation window). No public test of indexer recovery from intentional state poisoning. |
| Severity | M. |
| Before-audit action | Document the indexer's confirmation depth + reorg-handling policy. |
| Before-mainnet action | Confirmation depth tuned for mainnet. Reorg-handling explicit. Alerting on indexer fallback / stall. |

## 10. Backend shadow / manual intent projection (Sepolia reconciliation)

| Field | Value |
|---|---|
| Status | The Sepolia reconciliation path uses a "shadow intent + transaction" projection so the backend can be brought up cold after a chain event and converge to the chain truth. |
| Security assumption | The shadow projection is **strictly read-only** with respect to the chain. It cannot send a chain transaction. It can only mutate backend-side DB rows. |
| Evidence | `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`. |
| Gap | The shadow / manual import is operator-only. The endpoints used to perform shadow imports are admin-gated; no public path can trigger them. This is correct behaviour, but the gate boundary must be documented. |
| Severity | M. |
| Before-audit action | Document the shadow projection surface + its public-vs-admin boundary explicitly. Show that `tests/e2e/no-admin-bearer.spec.ts` enforces no admin-test URL from the browser runtime. |
| Before-mainnet action | Shadow projection should NOT be available on the mainnet backend by default; if needed, gated behind explicit operator approval per write. |

## 11. Backend broadcast gates

| Field | Value |
|---|---|
| Status | Default config has `EXECUTOR_REAL_BROADCAST_ENABLED=false`. A single approval-gated retry on 2026-06-12 demonstrated the full pipeline; after that, returned to `false`. |
| Security assumption | The backend cannot broadcast without an env flip + an operator signature. Single-flip suffices for a closed run window. |
| Evidence | `E2E_SEPOLIA_LIVE_BROADCAST_RETRY_RESULT.md`, `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`. |
| Gap | No automated audit log of every flag-flip. Currently relies on the operator's manual log. |
| Severity | L on testnet; M on mainnet. |
| Before-audit action | Document the flag-flip runbook + the operator's logging discipline. |
| Before-mainnet action | Automated audit log + alerting on flag flip + multi-operator approval per flip. |

## 12. Frontend no-admin-bearer guarantee

| Field | Value |
|---|---|
| Status | `tests/e2e/no-admin-bearer.spec.ts` asserts no `Authorization` header is attached to any backend XHR from the app runtime on any navigation. Public-beta footer DOM scanned for bearer/RPC/DB-shape values. |
| Security assumption | The browser bundle cannot leak an admin credential because the admin endpoints are not consumed by the frontend at all. |
| Evidence | Spec file above; admin route under `src/app/admin/*` is for operator visibility only; it does not consume admin-secret-bearing endpoints. |
| Gap | None known at the testnet beta moment. |
| Severity | I. |
| Before-audit action | Cite the spec in the handoff index. |
| Before-mainnet action | Same as testnet — operator dashboard should remain isolated; admin endpoints should remain unreachable from any public origin. CORS hardening on the production backend. |

## 13. Public beta no-real-funds / unaudited messaging

| Field | Value |
|---|---|
| Status | Every trading route renders the sticky `TestnetUnauditedBanner` + `MainnetDisabledBanner` (when applicable) + the public-beta footer with safety bullets. Every public-beta doc leads with the "testnet only / no real funds / unaudited" disclaimer. |
| Security assumption | A tester cannot reasonably misinterpret the beta as a mainnet product. |
| Evidence | Frontend banner specs, public-beta READMEs. |
| Gap | None known — covered by positive-claim drift scan in every milestone. |
| Severity | I. |
| Before-audit action | Cite the banner spec + the disclaimer copy in the audit handoff index. |
| Before-mainnet action | The disclaimers will need to be **changed** for a mainnet release. The change must NOT happen until external audit closure + custody policy closure. |

## 14. Env / secrets handling

| Field | Value |
|---|---|
| Status | Backend `.env` is operator-only, mode `644`, not committed. Frontend `.env.local` is gitignored. No bearer / RPC URL with key / DATABASE_URL appears in any committed file. Sensitive-string scan enforced in every milestone. |
| Security assumption | Secrets stay out of git history. Operator-side rotation procedures exist (informal). |
| Evidence | RUN_STATE.md milestone validations; `.env` mtime preserved across milestones. |
| Gap | No formal secret-rotation policy doc. No automated `.env`-leak detection in CI. |
| Severity | M. |
| Before-audit action | Document the operator's secret-rotation policy. Add a pre-commit hook or CI step that fails on `.env` patterns. |
| Before-mainnet action | KMS-managed secrets. Per-secret rotation cadence. Compromise IR runbook. No `.env` file in production at all. |

## 15. Production signer / AWS-KMS — OUT OF SCOPE for this packet

| Field | Value |
|---|---|
| Status | NOT BUILT. Documented in `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`, `AWS_KMS_OPERATOR_SETUP_PACK.md`, `BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` as the target model. |
| Security assumption | Until built, the executor is a plain testnet EOA. |
| Evidence | The result docs above describe what would be built. |
| Gap | Entire subsystem deferred. |
| Severity | C if mainnet were attempted today; I at testnet beta. |
| Before-audit action | An external audit could review the **design** but not the implementation. Operator must decide whether to scope KMS into the audit or do it post-audit. |
| Before-mainnet action | Full KMS-backed signer + adapter test coverage + CloudTrail / monitoring + IAM policy review. |

## 16. Safe / governance production flows — OUT OF SCOPE for this packet

| Field | Value |
|---|---|
| Status | NOT BUILT. Documented in `GOVERNANCE_*` docs in `deopt-v2-sol/docs/`. Target model in `MAINNET_CUSTODY_POLICY.md §R-1..R-9` (GOVERNANCE_MULTISIG ≥3-of-5; OPS_MULTISIG ≥2-of-3 OR 3-of-5 pending `Q-CD-2`). |
| Security assumption | Until built, ownership is a single EOA. |
| Evidence | Custody policy + governance docs. |
| Gap | Entire subsystem deferred. |
| Severity | C if mainnet were attempted today; I at testnet beta. |
| Before-audit action | Document the target governance model in the audit handoff index. |
| Before-mainnet action | Full Safe deployment + ownership transfer + timelock + tested-on-fork dry runs. |

---

## Severity legend recap

| Code | Meaning |
|---|---|
| I | Informational — no action required at the snapshot. |
| L | Low — should be tracked; can be deferred. |
| M | Medium — must be addressed before audit dispatch or before mainnet, depending on the row. |
| H | High — must be addressed before audit dispatch. |
| C | Critical — outright blocks mainnet; explicit "do not ship" boundary. |

Severity ratings here are deliberately the operator's self-assessment, not audit findings. An external auditor may reclassify any row.

---

**End of security re-anchor overview.**
