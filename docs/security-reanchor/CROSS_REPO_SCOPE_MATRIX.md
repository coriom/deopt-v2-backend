# DeOpt V2 — Cross-Repo Scope Matrix

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

What is in scope for a future external security review, what is out of scope, broken down per repo. Reviewer can use this to plan a scope letter; operator can use this to know what should NOT be promised.

---

## Conventions

| Tag | Meaning |
|---|---|
| **IN-SCOPE-NOW** | Frozen + buildable + would be presented to a reviewer today. |
| **IN-SCOPE-WITH-FIX** | Belongs in scope but a gap (test, doc, hardening) must close first. |
| **OUT-OF-SCOPE-DEFER** | Deliberately deferred; not part of this testnet beta. |
| **OUT-OF-SCOPE-FOREVER** | Not a DeOpt V2 concern (e.g. underlying wallet code, RPC provider, OS). |

---

## 1. Solidity (`deopt-v2-sol/`)

### IN-SCOPE-NOW

| Contract / artefact | Scope notes |
|---|---|
| `OptionMatchingEngine` | EIP-712 signature verification + nonce handling + `executeTrade` entry point. |
| `MarginEngine` | Per-account margin accounting. Bidirectional auth with `OptionMatchingEngine`. |
| `CollateralVault` + `CollateralVaultViews` | ERC-20 deposit / withdraw / settle accounting. |
| `OptionProductRegistry` | Product + series creation + view surface. |
| `OracleRouter` | `getPriceSafe` + `maxDelay=60s` semantics. |
| `FeesManagerV2` + `ProtocolFeeVault` | Fee split + accrual + sweep. |
| `MarginEngineLens` | Read-only composed views. |
| `InsuranceFund` | Insurance accounting (testnet posture). |
| `RiskModule` | IM / MM calc. |
| ABI freeze artefacts (`abis/freeze-v2-product-rc1/*`) | The selectors + storage layout that any future PR must not break. |

### IN-SCOPE-WITH-FIX

| Item | Gap that must close before audit |
|---|---|
| Solidity test inventory | No `test/*.t.sol` files located. **Action:** publish the actual test inventory + coverage delta — see `PRE_AUDIT_ACTION_PLAN.md` item 1. |
| Invariant docs | Need `INVARIANTS.md` enumerating: nonce monotonicity, vault accounting, fee accounting, position-quantity zero-sum, oracle freshness gate. |
| Storage-layout drift detection | The `.txt` snapshot exists; needs a CI hook that diffs on every PR. |
| `MockPriceSource` boundary | Used on Sepolia; must be flagged "test-only, NOT deployed to mainnet" in the scope letter to avoid auditor confusion. |

### OUT-OF-SCOPE-DEFER

| Item | Why |
|---|---|
| `PerpEngine*`, `PerpMatchingEngine`, `PerpMarketRegistry`, `PerpRiskModule`, `PerpEngineLens` | Perp surface deferred per `Q-CD-6` decision (NOT_APPLICABLE_AT_LAUNCH). |
| `FeesManager.sol` V1 | Superseded by V2. |
| Mainnet redeployment scripts | None exist; this packet does not produce mainnet deployment artefacts. |
| Governance multisig + timelock contracts (`GnosisSafe`, `TimelockController` integration) | Deferred until mainnet readiness path opens. Documented in `GOVERNANCE_*` docs as design only. |

### OUT-OF-SCOPE-FOREVER

| Item | Why |
|---|---|
| Solidity compiler bugs | Use the documented `solc` version; audit reviews the source, not the compiler. |
| Base / OP-Stack rollup-level bugs | Out of project scope. |
| ERC-20 implementation bugs in `mUSDC` | mUSDC is a test mock; mainnet would use real USDC. |
| Wallet vendor signing-prompt UX bugs | Wallet code is third-party. |

---

## 2. Backend (`deopt-v2-backend/`)

### IN-SCOPE-NOW

| Surface | Scope notes |
|---|---|
| Public OpenAPI surface | `docs/openapi/trading-api.openapi.json` (version `0.1.0-mvp`). 13 public paths, zero admin paths. |
| Public trading endpoints | Products / series / quote preview / portfolio / balances / positions / history / health. |
| Public intent-creation endpoint | `POST /options/execution-intents` (M-P2f). NO signer call. NO broadcast call. |
| Signing-payload + signatures endpoints | Returns the EIP-712 envelope; accepts a signer's signature. NOT a broadcast trigger. |
| Status envelope semantics | `ok / partial / stale` + per-request `warnings` array. |
| Error envelope | Stable codes: `INVALID_ADDRESS / SOURCE_UNAVAILABLE / RPC_UNAVAILABLE / ORACLE_UNAVAILABLE / ACCOUNT_STATE_UNAVAILABLE / SETTLEMENT_PREVIEW_UNAVAILABLE / CONFIG_MISSING`. |
| Event indexer worker | `OPTION_EVENT_INDEXER_ENABLED=true`. |
| Reconciliation worker | `OPTION_RECONCILIATION_WORKER_ENABLED=true`. |
| Backend Postgres schema | `option_execution_intents / _transactions / _events / _reconciliations`, `option_event_indexer_state`. |
| Execution gate env vars | `EXECUTION_ENABLED`, `EXECUTOR_REAL_BROADCAST_ENABLED`, `OPTION_EXECUTION_BROADCAST_ENABLED` — default false. |

### IN-SCOPE-WITH-FIX

| Item | Gap |
|---|---|
| Test inventory + coverage delta | ~49 `mod tests` declarations; need a one-pager summarising what's covered + what's not (especially around the broadcast-disabled startup path). |
| Shadow-projection / manual-import API boundary | Explicit `admin/` gating works; needs a one-page doc that an auditor can read without spelunking the codebase. |
| Backend-side signature-verification posture | Backend-side verification is currently disabled (chain-side verification still enforced); this is intentional but must be explicitly documented for the auditor. |
| Indexer reorg-handling policy | Need a written confirmation-depth + reorg-recovery contract. |

### OUT-OF-SCOPE-DEFER

| Item | Why |
|---|---|
| AWS KMS signer integration | NOT BUILT for production use. Local executor key only at testnet. |
| Mainnet broadcast path | NOT BUILT. Documented as design only in `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md`. |
| Production monitoring + alerting (full stack) | Partial design in `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md`; not deployed. |
| Production gas / fee rebate policy | Drafted in `BACKEND_GAS_FEES_REBATES_POLICY_V1.md`; not active. |
| Live-broadcast flag-flip runbook hardening | Drafted in `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md`; not hardened. |
| Vendor-specific KMS adapter implementations | Pluggable adapter shipped (`BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`) but vendor-specific cuts are deferred. |
| Bug-bounty program rules | Not active; no rules to scope. |

### OUT-OF-SCOPE-FOREVER

| Item | Why |
|---|---|
| Postgres engine bugs | Use the documented version; trust the engine. |
| Axum / Tokio bugs | Third-party. |
| RPC provider availability | Out of project scope. |

---

## 3. Frontend (`deopt-v2-frontend/`)

### IN-SCOPE-NOW

| Surface | Scope notes |
|---|---|
| Trading routes | `/`, `/markets`, `/markets/[productId]`, `/portfolio`, `/history`, `/health`, `/transactions/[requestId]`. |
| Wallet provider + EIP-1193 integration (viem) | `src/lib/wallet.tsx`. |
| EIP-712 typed-data signing flow | `src/lib/eip712.ts` + `src/components/trading/TradeTicket.tsx`. |
| Mainnet hard-stop | `chains.ts::isMainnetEnabled() === false` + banner + `signTypedData()` refusal. |
| Wrong-network blocker | `WrongNetworkBanner` + `TradeTicket.canSign` gate. |
| Public-beta footer + link config | `src/components/PublicBetaFooter.tsx` + `src/lib/public-beta-links.ts`. |
| Sign-failure CTA | `SigningStateModal` "Report this issue" path. |
| No-admin-bearer guarantee | `tests/e2e/no-admin-bearer.spec.ts`. |
| State semantics | `LoadingState / EmptyState / ErrorState / StaleDataBadge` + per-error-code friendly hints. |
| Playwright spec suite | 30 tests in 12 files. |
| Build artefacts | `npm run build` green; 9 routes (7 static + 2 dynamic). |

### IN-SCOPE-WITH-FIX

| Item | Gap |
|---|---|
| Operator-side admin dashboard (`src/app/admin/*`) | Not part of the public testnet beta user flow but DOES exist in the bundle. Must either be explicitly scoped IN (operator only) or moved out of the public bundle before an audit. **Action:** decide. |
| Public-beta link config — placeholder fill | Six `{{PUBLIC_BETA_*_URL}}` placeholders remain (intentionally). Audit can review the gating semantics; the URL fill is a separate operator action. |
| Playwright execution proof | Spec graph passes `--list`; targeted execution skipped in WSL2 sandbox. Need a CI run that exercises the suite on a Linux box with `libnspr4` available. |

### OUT-OF-SCOPE-DEFER

| Item | Why |
|---|---|
| Mainnet UI variant | Will not exist until external audit + custody closure. |
| `wallet_addEthereumChain` push for Base Sepolia | Deliberately omitted (we do not push custom RPC URLs at the user). |
| Frontend-initiated `executeTrade` | Architecturally forbidden; executor side handles broadcast. |

### OUT-OF-SCOPE-FOREVER

| Item | Why |
|---|---|
| Wallet extension bugs | Third-party. |
| Browser vendor bugs | Third-party. |

---

## 4. Project root + repo-spanning docs

### IN-SCOPE-NOW

| Doc | Purpose |
|---|---|
| `RUN_STATE.md` | Macro execution state, milestone history. |
| `TESTNET_RUNBOOK.md` | Testnet operator procedures. |
| `MAINNET_CUSTODY_POLICY.md` | Custody design (pre-cutover). Used by the audit handoff as the target ownership model. |
| `MAINNET_CUSTODY_DECISIONS_ADDENDUM_TEMPLATE.md` | Records open `Q-CD-*` decisions. |
| `docs/public-beta/*` (15 files in backend repo) | Public-facing posture. |
| `docs/security-reanchor/*` (this packet) | Security re-anchor. |
| `docs/EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` | Follow-up brief. |
| `docs/MAINNET_LAUNCH_READINESS_NEXT_TASK.md` | Follow-up brief. |
| `docs/MAINNET_AUDIT_HANDOFF_INDEX_FINAL.md` | Existing audit handoff index — re-anchored by §7 of this matrix. |
| `docs/COMMUNITY_FEEDBACK_LOOP_RESULT.md` | 2026-06-12 milestone outcome. |
| `docs/FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md` | 2026-06-12 milestone outcome. |
| `docs/SEPOLIA_*_RESULT.md` (multiple) | Sepolia broadcast + reconciliation history. |

### IN-SCOPE-WITH-FIX

| Doc | Gap |
|---|---|
| `BACKEND_EXECUTOR_CUSTODY.md` | Marked "mostly TODOs" in inventory. Needs to be filled (target rather than placeholder). |
| Open `Q-CD-*` decisions in custody policy | `Q-CD-2` (OPS multisig threshold), `Q-CD-5` (KMS vendor), `Q-CD-6` (perp scope). Should be resolved (or explicitly deferred with date) before audit dispatch. |

### OUT-OF-SCOPE-DEFER

| Item | Why |
|---|---|
| Mainnet outreach copy | Drafted as design only; not used until mainnet path opens. |
| `MAINNET_AUDIT_OUTREACH_DRAFT.md` (if present) | Outreach is a separate later milestone. Do not send anything from this packet. |

### OUT-OF-SCOPE-FOREVER

| Item | Why |
|---|---|
| Operator's private custody material (`~/DEOPT/private/**`) | Private. Not committed. Not read. Not in scope. |

---

## 5. What an external auditor would see

If an auditor were engaged today (they are NOT), the scope letter would say:

> **In scope:**
> * The 11 frozen Solidity contracts under `deopt-v2-sol/abis/freeze-v2-product-rc1/`.
> * The public backend OpenAPI surface (13 paths, no admin).
> * The frontend trading UI surface (public-beta posture, mainnet hard-stop, wrong-network blocker).
> * The EIP-712 signature model + nonce model + oracle gate at the contract level.
> * The vault + fee + margin accounting at the contract level.
>
> **Out of scope:**
> * AWS KMS production signer (not built).
> * Safe multisig / governance production flows (not built).
> * Production monitoring / alerting / incident response.
> * Mainnet deployment (no addresses exist).
> * The perp surface (deferred).
> * Operator runbooks (target documents, not products).

A reviewer would expect to see:
* `PRODUCT_FREEZE_SUMMARY.md` (this packet)
* `SECURITY_REANCHOR_OVERVIEW.md` (this packet)
* `INVARIANTS.md` (to be written per `PRE_AUDIT_ACTION_PLAN.md`)
* Source commit + tag pointer
* Storage-layout snapshot
* Test inventory + coverage delta
* Threat-model write-up
* Known-issues list (with the legacy stale ME explicitly flagged)
* Out-of-scope list (same as §1.OUT-OF-SCOPE-DEFER above)

---

**End of cross-repo scope matrix.**
