# Next-task prompt: E2E-SEPOLIA-TRADING-LIFECYCLE (M-P5)

Copy/paste this prompt verbatim to initiate M-P5. **M-P5 is NOT yet
ready** — it gates on M-P4b (Playwright + cycler) + M-P2c (on-chain
RPC orchestration) closure. This file is a forward placeholder.

---

```
Workspace root is ~/DEOPT.

Execute E2E-SEPOLIA-TRADING-LIFECYCLE only.

This is M-P5 of the product-readiness roadmap. The goal is to run
the full DeOpt V2 trading lifecycle against Base Sepolia rehearsal
infrastructure, validating that the M-P4 local lifecycle composes
under real L2 + RPC + indexer + reconciliation conditions.

Hard prerequisites (ALL MUST be closed):
  - M-P4 closed (E2E local lifecycle).
  - M-P4b closed (Playwright + mock-status cycler).
  - M-P2c closed (on-chain RPC orchestration for 6 trading endpoints).

If any of the above is not yet closed, STOP and surface the gap.

Do not deploy to mainnet.
Do not broadcast to mainnet.
Do not send Sepolia transactions OUTSIDE the rehearsal scenarios
documented below. Each Sepolia tx must be explicit + approved in the
scenario script.
Do not create Safe transactions.
Do not create AWS resources.
Do not edit production `.env` (use `.env.sepolia.local`).
Do not expose secrets.
Do not touch mainnet.
Do not expose admin Bearer to trading UI.

Strategic context:

External audit deferred until M-P7 closure. M-P5 is the final pre-
public-beta confidence step before M-P6 (public docs beta pack)
opens the rehearsal to outside testers.

Goal:

Run the orderbook 9-step scenario + RFQ 7-step scenario + 10-row
failure-case sweep from `E2E_TRADING_LIFECYCLE_TEST_PLAN.md §2`
against:
  - Base Sepolia chain id 84532;
  - operator-managed Sepolia RPC URL;
  - Sepolia-deployed contract addresses (from M-P4 rehearsal);
  - backend pointed at Sepolia + Postgres;
  - frontend pointed at backend with NEXT_PUBLIC_CHAIN_ENV=sepolia;
  - Playwright harness from M-P4b adapted for Sepolia.

Verify:
  - R5 drift = 0 across the full scenario sweep;
  - indexer caught up at end (lag < 5 blocks);
  - reconciliation drift = 0;
  - Cluster 4 launch invariant: PFV.rebateReserve(asset) === 0 +
    FM_V2.rebateBudget(asset) === 0;
  - signer policy: all broadcasts succeeded via the in-process k256
    signer (Sepolia rehearsal only); no remote signer required.

Required Phase A — inspect:
  - `~/DEOPT/deopt-v2-backend/docs/E2E_TRADING_LIFECYCLE_TEST_PLAN.md §2`;
  - `~/DEOPT/deopt-v2-backend/docs/E2E_LOCAL_TRADING_LIFECYCLE_RESULT.md`;
  - `~/DEOPT/deopt-v2-backend/docs/BACKEND_TRADING_API_PHASE_2_RESULT.md`;
  - `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_TRADING_SIGNING_RESULT.md`;
  - operator-side Sepolia rehearsal evidence at
    `~/DEOPT/deopt-v2-sol/docs/V2G_GOV_G_RESULT.md` (anchored addresses).

Required Phase B — environment:
  - Create `.env.sepolia.local` (gitignored) with operator-supplied
    Sepolia RPC URL + rehearsal-only Sepolia executor private key
    + Sepolia contract addresses.
  - **NEVER** commit `.env.sepolia.local`.
  - Sepolia executor private key MUST be a rehearsal-only key; NOT
    a key that holds real funds anywhere.
  - Mainnet defence-in-depth: backend startup MUST refuse to
    accept `EXECUTOR_PRIVATE_KEY` when `chain_id == 8453` (already
    tested by `validate_signer_backend`).

Required Phase C — scenarios:
Execute the scenarios from `E2E_TRADING_LIFECYCLE_TEST_PLAN.md §2`:
  - 9-step orderbook lifecycle;
  - 7-step RFQ lifecycle;
  - 10-row failure case sweep.

Required Phase D — verification:
  - R5 drift = 0 (CV.balances(PFV, asset) - PFV.feeBalance -
    PFV.rebateReserve === 0).
  - `GET /reconciliation/status` → drift = 0.
  - `GET /indexer/status` → lag < 5 blocks.
  - Cluster 4 launch invariant pin.

Required Phase E — result doc + RUN_STATE:
Create
`~/DEOPT/deopt-v2-backend/docs/E2E_SEPOLIA_TRADING_LIFECYCLE_RESULT.md`:
  - environment summary (Sepolia chain id; RPC reachable; contracts
    anchored);
  - scenario pass / fail per step;
  - failure-case pass / fail per row;
  - R5 drift final value;
  - reconciliation result;
  - tx hash + block list (operator-public-safe);
  - blockers remaining for M-P6.

Update `~/DEOPT/RUN_STATE.md` with concise closure paragraph.

Validation:
  - All Playwright specs from M-P4b run green against Sepolia
    rehearsal.
  - `cargo test`, `npx next build` clean.
  - `git diff --check`, `git status`.
  - Sensitive-string scan: NO mainnet contract addresses; NO
    production secrets; Sepolia RPC URL allowed in `.env.sepolia.local`
    ONLY; tx hashes allowed in result doc.

Forbidden:
  - no mainnet tx;
  - no live broadcast against any chain OTHER than Sepolia for the
    specific tx documented in the scenarios;
  - no Safe tx;
  - no governance mutation;
  - no fund movement OUTSIDE the rehearsal scenarios;
  - no production `.env` edit;
  - no AWS resource creation;
  - no KMS key creation;
  - no real AWS account IDs / KMS key IDs / KMS ARNs;
  - no guessed mainnet executor address;
  - no production signer address guess;
  - no invented mainnet contract addresses;
  - no audited claim;
  - no mainnet-ready claim;
  - no admin Bearer in trading UI.

Hard stops:
  - stop if any scenario step would require a mainnet tx;
  - stop if R5 drift becomes non-zero (file as a regression);
  - stop if reconciliation drift becomes non-zero;
  - stop if indexer falls > 5 blocks behind and doesn't catch up;
  - stop if a Sepolia tx fails with a revert other than the
    deliberately-induced failure-case reverts;
  - stop if Playwright cannot reach the Sepolia backend;
  - stop if a wallet popup asks for mainnet tx;
  - stop if `validate_signer_backend` refuses startup unexpectedly.

Return final report grouped by:
workspace,
docs/source inspected,
environment,
orderbook scenario,
RFQ scenario,
failure-case sweep,
R5 drift,
reconciliation,
Cluster 4 launch invariant,
tx hashes + blocks,
RUN_STATE update,
files changed,
validations,
blockers remaining,
next milestone recommendation (M-P6 public docs beta pack).
```

---

**End of next-task prompt.**
