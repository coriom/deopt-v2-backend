# DeOpt V2 — Updated Risk Register

> **Snapshot date:** 2026-06-12. **Posture:** public testnet beta, Base Sepolia only, unaudited, not mainnet-ready.

Refresh of the product / security risk register at the testnet-beta moment. This document supersedes earlier `MAINNET_AUDIT_RISK_REGISTER_*` draft tables for the purposes of audit handoff.

## Conventions

| Field | Meaning |
|---|---|
| **R-ID** | Stable risk id used in cross-references. |
| **Likelihood** | L1 (rare) — L5 (near-certain) at the snapshot. |
| **Impact** | I1 (informational) — I5 (catastrophic for testnet beta scope; for mainnet impact see `MAINNET_READINESS_GAP_ANALYSIS.md`). |
| **Residual** | After current mitigations. Severity codes from `SECURITY_REANCHOR_OVERVIEW.md`: I / L / M / H / C. |
| **Status** | OPEN / MITIGATED / ACCEPTED / DEFERRED. |

Impact and likelihood are at **testnet beta scope**. Mainnet impact would be uniformly higher and is tracked separately.

---

## R-1 — Unaudited protocol

* **Description:** The contracts have not been reviewed by an external auditor. Internal review has happened.
* **Likelihood:** L5 (it is, definitionally).
* **Impact:** I3 at testnet (no real funds); I5 if mainnet were attempted.
* **Mitigations:** Public-beta posture documented in every disclaimer. Frontend mainnet hard-stop. `KNOWN_LIMITATIONS_AND_RISKS.md` §1 states "Not audited" explicitly.
* **Residual:** H — must be resolved before mainnet.
* **Status:** OPEN (this packet prepares the handoff; the audit itself is a later milestone).

## R-2 — Mock-oracle / testnet-oracle assumptions

* **Description:** Testnet uses `MockPriceSource` controlled by the operator. Operator can push any price during the testnet beta.
* **Likelihood:** L4 (oracle staleness happens routinely on testnet).
* **Impact:** I2 (only causes refusal of trades; never causes wrong settlement on testnet because the maxDelay gate refuses stale prices).
* **Mitigations:** `maxDelay = 60 s`. `getPriceSafe` returns `ok=false` on staleness. Trade reverts on stale price.
* **Residual:** M on testnet; C on mainnet.
* **Status:** ACCEPTED on testnet; OPEN for mainnet.

## R-3 — Short oracle `maxDelay` / stale-price refusal cliff

* **Description:** The 60-second `maxDelay` means many quote previews return `partial` with `ORACLE_UNAVAILABLE`. UX confusion is expected.
* **Likelihood:** L4.
* **Impact:** I2 (UX friction only).
* **Mitigations:** Friendly stale-oracle copy in `QuotePreviewCard`. Explicit `KNOWN_LIMITATIONS_AND_RISKS.md` callout. Public-beta `FAQ.md` covers "Why does my quote say stale oracle?".
* **Residual:** L.
* **Status:** ACCEPTED (UX trade-off; testnet beta posture is conservative on oracle freshness).

## R-4 — Executor key risk

* **Description:** Executor EOA private key is operator-held. On testnet this is acceptable; on mainnet it would be a single point of failure.
* **Likelihood:** L2 (testnet); L3 (mainnet).
* **Impact:** I3 at testnet; I5 at mainnet.
* **Mitigations:** Single-flip `EXECUTOR_REAL_BROADCAST_ENABLED` gate. Operator manual log of broadcasts. Public funds at zero risk because public broadcast disabled by default.
* **Residual:** M at testnet; C at mainnet.
* **Status:** OPEN; resolution is the KMS cutover documented in `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` and `AWS_KMS_OPERATOR_SETUP_PACK.md`.

## R-5 — Owner key risk

* **Description:** Contract owners are single EOAs on testnet. Compromise would let an attacker reconfigure / pause / drain (subject to contract logic).
* **Likelihood:** L1 at testnet (limited blast radius).
* **Impact:** I4 at testnet; I5 at mainnet.
* **Mitigations:** Testnet is sandbox; testnet owner = testnet operator. No mainnet ownership exists.
* **Residual:** M at testnet; C at mainnet.
* **Status:** OPEN; resolution per `MAINNET_CUSTODY_POLICY.md §R-1..R-9` GOVERNANCE_MULTISIG (≥3-of-5) + timelock.

## R-6 — Backend / indexer reconciliation drift

* **Description:** Indexer may lag the chain. Lifecycle endpoint may briefly show `missing_events`. Recovery is automatic but observable.
* **Likelihood:** L3.
* **Impact:** I2 (UX friction only; chain is the source of truth).
* **Mitigations:** Reconciliation worker. Manual operator force-tick path. Documented in `SEPOLIA_BACKEND_RECONCILIATION_FIX_RESULT.md`.
* **Residual:** L.
* **Status:** ACCEPTED.

## R-7 — Manual / shadow intent projection misuse

* **Description:** The operator-side shadow projection endpoint exists to recover the backend DB after a chain event. If accessible from a public origin it would let an attacker forge backend state.
* **Likelihood:** L1 (admin-gated).
* **Impact:** I3 (backend lies; chain state unaffected).
* **Mitigations:** `/admin/*` router gated by `ADMIN_API_REQUIRE_TOKEN`. Frontend never attaches `Authorization`. Browser-runtime `/admin/test/*` fetch forbidden by `no-admin-bearer.spec.ts`.
* **Residual:** L.
* **Status:** MITIGATED.

## R-8 — Stale deployment address risk

* **Description:** Legacy stale `OptionMatchingEngine 0xf2D1D85…` is still on chain. Trades signed against it cannot settle (the matching engine isn't authorized on the canonical MarginEngine).
* **Likelihood:** L2 (could happen if a user follows outdated docs).
* **Impact:** I2 (trade reverts; no funds lost on testnet because they're testnet).
* **Mitigations:** Canonical addresses documented in `CONTRACT_ADDRESSES_BASE_SEPOLIA.md` with explicit "DO NOT USE" callout for the legacy address. `FAQ.md §Q15` covers it. Frontend reads the canonical address from backend config.
* **Residual:** L on testnet; would be H on mainnet (no equivalent exists on mainnet — clean slate).
* **Status:** MITIGATED.

## R-9 — Frontend wrong-network risk

* **Description:** User connects a wallet on the wrong network and somehow signs a payload that targets a different chain.
* **Likelihood:** L1.
* **Impact:** I2.
* **Mitigations:** `WrongNetworkBanner` + `MainnetDisabledBanner` + `signTypedData` refusal + `TradeTicket.canSign` gate. Tests `wrong-network-banner.spec.ts` + `mainnet-disabled.spec.ts`.
* **Residual:** L.
* **Status:** MITIGATED.

## R-10 — Admin bearer leakage

* **Description:** A bearer token used by the operator dashboard could leak into a public XHR, screenshot, or bug report.
* **Likelihood:** L1.
* **Impact:** I4 (operator-side; depends on what the bearer authorizes).
* **Mitigations:** `no-admin-bearer.spec.ts` enforces zero `Authorization` header from app runtime. `BUG_REPORT_TEMPLATE.md §1` forbids tester from sharing it. `OPERATOR_PUBLIC_BETA_URLS_FILL.md §3` forbids bearer in any link URL.
* **Residual:** L.
* **Status:** MITIGATED.

## R-11 — API `partial` / `SOURCE_UNAVAILABLE` confusion

* **Description:** Backend returns `partial` or `SOURCE_UNAVAILABLE` during testnet warm-up. Developers integrating against the API may misinterpret as failure.
* **Likelihood:** L3.
* **Impact:** I2 (integrator friction).
* **Mitigations:** Status envelope semantics documented in `DEVELOPER_API_GUIDE.md`. Friendly hints in `ErrorState.hintForCode()`.
* **Residual:** L.
* **Status:** ACCEPTED.

## R-12 — Liquidity / market-maker absence

* **Description:** No active market makers on the testnet beta. Quotes may be artificial; orderbook may be empty.
* **Likelihood:** L5 (it is the current state).
* **Impact:** I1 at testnet beta (it's a feedback phase, not a liquidity phase).
* **Mitigations:** Documented in `KNOWN_LIMITATIONS_AND_RISKS.md`. Public-beta posture is "feedback phase, not trading phase".
* **Residual:** I.
* **Status:** ACCEPTED.

## R-13 — Public beta confusion (mistaken-for-mainnet)

* **Description:** A reader / press / community member could misread the beta as a mainnet launch.
* **Likelihood:** L2.
* **Impact:** I3 (reputational).
* **Mitigations:** Public-beta vocabulary enforced in every milestone. Every announcement draft has an honesty checklist. Banners on every trading route. Positive-claim drift scan in every milestone.
* **Residual:** L.
* **Status:** MITIGATED.

## R-14 — Documentation drift

* **Description:** Docs may go stale as code changes.
* **Likelihood:** L3.
* **Impact:** I2 (reader misled).
* **Mitigations:** Result docs cite the source commit. Public-beta docs cite the canonical contract addresses. RUN_STATE.md milestone history.
* **Residual:** L.
* **Status:** OPEN (intrinsic to the lifecycle; manage via review cadence).

## R-15 — Secrets hygiene

* **Description:** A secret leaks into git history, a public log, a screenshot, or a bug report.
* **Likelihood:** L1.
* **Impact:** I4.
* **Mitigations:** `.gitignore` for `.env`. Sensitive-string scan in every milestone. `BUG_REPORT_TEMPLATE.md §1` rules. `OPERATOR_PUBLIC_BETA_URLS_FILL.md §3` rules.
* **Residual:** L.
* **Status:** MITIGATED (but never closed; ongoing discipline).

## R-16 — Production signer / KMS not cut over

* **Description:** Production signer is testnet EOA. Mainnet cutover not done.
* **Likelihood:** L5 (it is the state).
* **Impact:** I0 at testnet beta (no mainnet exposure); C if mainnet were attempted.
* **Mitigations:** Mainnet path explicitly closed. `EXECUTOR_REAL_BROADCAST_ENABLED=false` default. `MAINNET_CUSTODY_POLICY.md` documents target. KMS adapter pluggable per `BACKEND_KMS_VENDOR_ADAPTER_IMPLEMENTATION_PLUGGABLE_RESULT.md`.
* **Residual:** C blocking mainnet.
* **Status:** DEFERRED (intentional).

## R-17 — Safe / governance not productionized

* **Description:** No Safe multisig. No timelock. No governance flow.
* **Likelihood:** L5 (it is the state).
* **Impact:** I0 at testnet beta; C at mainnet.
* **Mitigations:** Documented in `GOVERNANCE_*` docs as target design.
* **Residual:** C blocking mainnet.
* **Status:** DEFERRED (intentional).

## R-18 — No bug bounty

* **Description:** No bounty program. Reporters' incentives are intrinsic only.
* **Likelihood:** L5 (it is the state).
* **Impact:** I2 (slower vulnerability surfacing).
* **Mitigations:** `FEEDBACK_TRIAGE_WORKFLOW.md §6` describes the security disclosure path. `BUG_REPORT_TEMPLATE.md` distinguishes public vs private path.
* **Residual:** M at audit-readiness time (auditor will ask about bounty); L at testnet beta.
* **Status:** DEFERRED (post-audit milestone).

## R-19 — No external audit yet

* **Description:** Audit firm not engaged. Scope letter not signed. SOW not drafted.
* **Likelihood:** L5 (it is the state).
* **Impact:** I5 if mainnet were attempted; I0 at testnet beta.
* **Mitigations:** This packet is the precursor. `EXTERNAL_AUDIT_DISPATCH_PREP_NEXT_TASK.md` is the next-task brief.
* **Residual:** H blocking mainnet.
* **Status:** OPEN (this packet does NOT launch outreach; that is a separate later milestone).

---

## Summary by severity (testnet-beta-now)

| Severity | Open risks |
|---|---|
| **C** | none currently active at testnet; latent: R-16, R-17 (deferred-on-purpose; would be C if mainnet attempted) |
| **H** | R-1, R-19 (audit). R-5 if mainnet attempted. |
| **M** | R-2, R-3, R-4 (deferred), R-6 (low residual), R-14, R-15 |
| **L** | R-7, R-8, R-9, R-10, R-11, R-13 |
| **I** | R-12 |

This is **operator self-assessment, not audit findings**. An external reviewer may reclassify any row.

---

**End of updated risk register.**
