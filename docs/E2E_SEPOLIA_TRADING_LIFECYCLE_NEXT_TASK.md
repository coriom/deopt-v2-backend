# E2E_SEPOLIA_TRADING_LIFECYCLE_NEXT_TASK (M-P5)

**Date written:** 2026-06-10
**Origin milestone:** M-P3c (`FRONTEND-CREATE-INTENT-UX`) — closed B3.
**Target milestone:** `E2E-SEPOLIA-TRADING-LIFECYCLE` (M-P5) — end-to-end
rehearsal on Base Sepolia (chain 84532) only.
**Posture:** **DRY-RUN FIRST. Operator-approval-gated for any live
broadcast.** **No mainnet.** **No Safe tx.** **No AWS/KMS creation.**
**No production `.env` edit.** **No real funds movement.** **No
audited-claim.**

---

## 1. Posture (read this first)

This milestone is **the first time** the DeOpt V2 stack exercises a
real public-chain transaction end-to-end. The posture is strict
because of that:

* **Sepolia only** (chain 84532). Mainnet (chain 8453) is permanently
  disabled in every code path and ALL four defence-in-depth gates
  must remain intact.
* **Dry-run first.** Every step is rehearsed against anvil + the
  M-P4c local-test cycler first; only after the dry-run passes is
  a live Sepolia broadcast considered.
* **Operator approval required** before any live Sepolia
  `eth_sendRawTransaction`. The operator approval lives outside this
  milestone (a separate sign-off doc).
* **No Safe tx.** Sepolia rehearsal uses the existing executor
  signer, not the production Safe-multi-sig flow.
* **No AWS / KMS creation.** Sepolia rehearsal uses the existing
  AWS/KMS configuration if already provisioned, OR the local
  private-key signer for the testnet rehearsal. **No new AWS
  resources, no new KMS keys, no production secrets are created.**
* **No production `.env` edit.** Any env values needed for the
  rehearsal go in `.env.sepolia` or a dedicated test config — never
  in a file that production reads.

## 2. Gates that must be green before M-P5 starts

| Gate | Source |
|---|---|
| B1 LOCAL_INTENT_FIXTURE_MISSING | closed (M-P4c) |
| B2 ON_CHAIN_RPC_NOT_WIRED | closed (M-P2e) |
| B3 FRONTEND_CREATE_INTENT_UX_MISSING | closed (M-P3c) |
| B5 BACKEND_TX_STATUS_FIXTURE_MISSING | closed (M-P4c) |
| Backend `cargo test --all-targets` 1182+ green | confirmed at M-P2e |
| Frontend `tsc + eslint + next build + playwright list` clean | confirmed at M-P3c |
| Mainnet hard-gate, 4 gates intact | confirmed at every milestone |

## 3. Scope — M-P5

### Phase A — Dry-run on anvil + M-P4c cycler

1. Anvil starts (chain 31337).
2. Backend starts with `OPTION_*_ADDRESS` env keys populated against
   anvil-deployed contracts.
3. Backend M-P4c fixture is enabled
   (`LocalTestFixturesConfig::enabled_for_chain_id(31337)`).
4. Frontend connects via the wallet fixture (Playwright).
5. Trade ticket: Create intent → Sign → Submit → Tx Status drives
   CREATED → BROADCAST → CONFIRMED via the cycler.
6. **Expected: all 21 Playwright specs pass + new M-P5 specs added.**

### Phase B — Sepolia rehearsal (dry-run only)

1. Backend `.env.sepolia` (NOT `.env`) populated with sepolia RPC URL
   + executor private key + the 5 sepolia contract addresses.
2. Backend started against Sepolia (chain 84532).
3. M-P4c fixture **disabled** for Sepolia (only used for anvil
   tests).
4. Frontend connects to backend; submits a quote-preview → create
   intent → fetch signing payload. **STOP before signing.**
5. Operator reviews the signing payload, the EIP-712 domain
   (chainId=84532), the verifying contract, and the tx envelope.
6. **Sign-off doc** required before proceeding.

### Phase C — Live Sepolia broadcast (operator-gated)

1. Operator approves the rehearsal.
2. User signs typed data in wallet on Sepolia.
3. Backend submits signature, operator broadcasts via existing
   executor.
4. Tx hash recorded; tx-status timeline drives through real
   on-chain confirmations.
5. R5 drift check + reconciliation check pass.
6. **Expected: zero unexpected reverts; zero unexpected fee
   movements; zero Safe-tx exposure; zero AWS/KMS resource creation.**

## 4. Forbidden in M-P5

* No mainnet broadcast.
* No Safe tx (Sepolia or otherwise).
* No production `.env` edit.
* No new AWS account / KMS key / IAM role creation.
* No new GitHub workflow that touches mainnet.
* No Solidity modification.
* No new ABI binding (Sepolia uses the frozen `v2-product-freeze-rc1`
  artefacts).
* No bypass of mainnet hard-gates.
* No "mainnet-ready" claim — that comes after M-P6/M-P7 + audit.

## 5. Hard stops

Stop and ask the user before proceeding if:

* Sepolia broadcast would require editing production `.env`.
* Sepolia broadcast would require a Safe tx.
* Sepolia signer mechanism is unclear or undocumented.
* R5 drift check is non-zero.
* Reconciliation check shows unexpected divergence.
* Any frontend code path could leak admin Bearer to the trading UI.
* Local setup would overwrite developer data.

## 6. R5 drift + reconciliation prerequisites

Before any live broadcast:

* `R5 drift = 0` (existing constraint from V2G-GOV-G).
* Backend reconciliation worker has run and reports `match` for the
  pre-rehearsal state.
* M-P4c local-test fixture is **disabled** on Sepolia (re-asserted at
  startup).

## 7. Deliverables for M-P5

* `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_DRY_RUN_RESULT.md`
* `docs/E2E_SEPOLIA_TRADING_LIFECYCLE_RUNBOOK.md`
* `docs/E2E_SEPOLIA_OPERATOR_SIGNOFF.md` (one-page approval template)
* Updated `RUN_STATE.md` with the closure paragraph.

## 8. Cross-links

* `BACKEND_TRADING_API_PHASE_5_RESULT.md` (M-P2e)
* `E2E_LOCAL_TX_STATUS_CYCLER_RESULT.md` (M-P4c)
* `~/DEOPT/deopt-v2-frontend/docs/FRONTEND_CREATE_INTENT_UX_RESULT.md` (M-P3c)
* `~/DEOPT/deopt-v2-frontend/docs/TRADING_CREATE_INTENT_FLOW_RUNBOOK.md` (M-P3c)
* `BACKEND_LIVE_BROADCAST_FLAG_FLIP_RUNBOOK_V2G_FX_Q1_C.md` (live broadcast safety)
* `BACKEND_SIGNER_CUTOVER_RUNBOOK_V2G_FX_Q1.md` (signer config)

**End of next-task prompt.**
