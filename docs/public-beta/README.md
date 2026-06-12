# DeOpt V2 — Base Sepolia Public Testnet Beta

> **Testnet only. No real funds. Unaudited. Experimental.** Not production. Not mainnet-ready. APIs and contracts may change without notice.

This directory is the public-facing documentation pack for the DeOpt V2 Base Sepolia testnet beta. It is meant for external testers, developer integrators, and curious community members.

---

## Read these in order

1. [PUBLIC_TESTNET_BETA_OVERVIEW.md](./PUBLIC_TESTNET_BETA_OVERVIEW.md) — what DeOpt V2 is, what this beta covers, what is and isn't ready.
2. [BASE_SEPOLIA_QUICKSTART.md](./BASE_SEPOLIA_QUICKSTART.md) — switch your wallet to Base Sepolia, get testnet ETH and mUSDC, run a sample trade.
3. [USER_TESTING_GUIDE.md](./USER_TESTING_GUIDE.md) — 10-step testing flow with expected outcomes.
4. [CONTRACT_ADDRESSES_BASE_SEPOLIA.md](./CONTRACT_ADDRESSES_BASE_SEPOLIA.md) — current Base Sepolia testnet addresses.
5. [DEVELOPER_API_GUIDE.md](./DEVELOPER_API_GUIDE.md) — backend API for integrators.
6. [KNOWN_LIMITATIONS_AND_RISKS.md](./KNOWN_LIMITATIONS_AND_RISKS.md) — important caveats.
7. [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md) — how to send us bug reports.
8. [FAQ.md](./FAQ.md) — common questions.

### Community feedback loop (2026-06-12)

9. [COMMUNITY_ONBOARDING.md](./COMMUNITY_ONBOARDING.md) — onboarding for external testers (join, try, report).
10. [BUG_REPORT_TEMPLATE.md](./BUG_REPORT_TEMPLATE.md) — public-safe bug report template with safety checklist.
11. [FEEDBACK_TRIAGE_WORKFLOW.md](./FEEDBACK_TRIAGE_WORKFLOW.md) — operator-facing triage playbook (classification, severity, escalation, pause criteria).
12. [PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md](./PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md) — pre/post-launch operator checklist.
13. [PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md](./PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md) — Discord / X / LinkedIn / README announcement drafts + pause-rollback template.
14. [OPERATOR_PUBLIC_BETA_URLS_FILL.md](./OPERATOR_PUBLIC_BETA_URLS_FILL.md) — checklist for swapping `{{PLACEHOLDER}}` tokens into real URLs.

---

## Public-beta launch checklist (for the operator)

| # | Item | Done? |
|---|---|---|
| 1 | Docs complete (this directory) | ✓ |
| 2 | App URL placeholder set (`{{APP_URL}}`) in user-facing docs | ✓ |
| 3 | Feedback channel docs ready (templates + triage workflow + operator-fill checklist) | ✓ (`COMMUNITY_FEEDBACK_LOOP_RESULT.md`, 2026-06-12); placeholders remain — see `OPERATOR_PUBLIC_BETA_URLS_FILL.md` |
| 4 | Faucet + mUSDC flow documented | ✓ |
| 5 | Known limitations published | ✓ |
| 6 | Testnet disclaimers visible in every doc | ✓ |
| 7 | Frontend shows testnet + unaudited banners + wrong-network + mainnet hard-stop + public-beta footer | ✓ (`FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md`, 2026-06-12) |
| 8 | Backend `/trading/health` endpoint available | ✓ (verified during reconciliation milestone) |
| 9 | Runbook for oracle refresh / reset exists | ✓ (`~/DEOPT/TESTNET_RUNBOOK.md` + internal `SEPOLIA_SETUP_FIXES_PACK_*` docs) |
| 10 | Operator knows how to recover / reset testnet environment | ✓ (internal docs) |

---

## What is NOT in this directory

* No mainnet addresses.
* No private RPC URLs.
* No admin bearer tokens or operator credentials.
* No production-only configuration.
* No claims of audit, mainnet-readiness, or safety for real funds.

---

## Next milestones (operator-side)

1. ~~`FRONTEND_TESTNET_LAUNCH_POLISH`~~ — DONE 2026-06-12 (`FRONTEND_TESTNET_LAUNCH_POLISH_RESULT.md`).
2. `COMMUNITY_FEEDBACK_LOOP` — wire up the placeholder feedback channels. **Frontend slots are now ready to receive real URLs** (see `deopt-v2-frontend/src/lib/public-beta-links.ts`).
3. `PRODUCT_FREEZE_AND_SECURITY_REANCHOR` — re-confirm the frozen ABI and prepare the security-review packet (still ahead of any external audit engagement).

See the `*_NEXT_TASK.md` briefs in `deopt-v2-backend/docs/`.

---

**End of public-beta README.**
