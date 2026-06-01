# V2G-PX — OPTION RFQ Live Readiness

## Status

- Milestone: **V2G-PX** — top-level live-readiness summary for the
  OPTION RFQ deploy + first smoke trade. **Docs-only.** No
  broadcast, no chain mutation.
- Date: 2026-06-01.
- Audience: human operator + governance reviewer scheduling the
  V2G-P broadcast window.
- Companion docs:
  - `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md` — step-by-step deploy/rewire procedure (last turn).
  - `deopt-v2-sol/docs/OPTION_RFQ_SMOKE_RUNBOOK_V2G_PX.md` — first-RFQ-trade smoke procedure.
  - `deopt-v2-backend/docs/OPTION_RFQ_OPERATOR_PACKET_V2G_PX.md` — packet-builder walkthrough for the operator.

---

## 1. Current live RFQ gap (re-confirmed)

Per `deployments/base-sepolia.manifest.draft.json` + the V2G-P0 audit:

| Surface | Status |
|---|---|
| FeesManagerV2 (`0x00dA0B…774f`) | ✅ live, V2 fees ON |
| Live MarginEngine (`0x287Cef…48Cc`) | ❌ lacks `applyRfqTrade` selector (`0x1ccdd23f`) — predates V2G-O |
| Live OptionMatchingEngine | ❌ **`null` in manifest** — never deployed on Base Sepolia |
| RFQ math (FeesManagerV2 `_effectiveRatePpm`) | ✅ live since V2G-N |
| OPTION RFQ Solidity flow (V2G-O bytecode) | ✅ offline ready; in `out/` |
| Backend RFQ typed-data primitives (V2G-P1) | ✅ live in PID 231297 (V2G-M2 restart picked up V2G-P0 backend surface; V2G-P1 packet builder is offline-ready in `target/`) |
| RFQ live support | ❌ **false** |

**State migration risk: LOW.** OPTION_MATCHING_ENGINE has never
been deployed → no nonces, no orders, no lifecycle rows to
migrate. The MarginEngine redeploy is non-destructive because all
OPTION-positions storage is local to the engine instance and the
new engine starts empty.

---

## 2. Strategy A — re-confirmed

| Strategy | Status | Why |
|---|---|---|
| **A — Redeploy MarginEngine + first-deploy OptionMatchingEngine + rewire dependents** | **SELECTED** | Cleanest; clear rollback to OLD MarginEngine if needed; no OPTION state to migrate. |
| B — Reuse live MarginEngine + first-deploy OptionMatchingEngine | rejected | Live MarginEngine lacks `applyRfqTrade` — `executeRfqTrade` would revert at the cast. |
| C — Deploy a separate RFQ adapter that proxies into live MarginEngine | rejected | Cannot reach `consumeFees(FlowKind.RFQ)` without the new internal helper. |
| D — Defer | rejected | Leaves a written-but-unexposed feature; drift risk. |

Recap of the deploy order (see deploy-rewire runbook for full
step-by-step):

```
1. Deploy            new MarginEngine V2G-O bytecode.
2. Deploy            new OptionMatchingEngine V2G-O bytecode.
3. Rewire            dependents → new MarginEngine.
4. Wire              FeesManagerV2.setFeeConsumer(new MarginEngine, true).
5. Activate          OPTION series via ConfigureMarkets.
6. Preflight smoke   read-only RFQ quote checks (FlowKind.RFQ).
7. First live RFQ    Tier 4 maker / Tier 2 taker through executeRfqTrade.
8. Verify            FeeChargedV2.flowKind=RFQ + FeeRebatedV2.flowKind=RFQ.
9. Manifest          update with new addresses + first RFQ tx.
```

---

## 3. What is ready (offline)

### 3.1 Solidity scripts

| Script | Purpose | Safe-by-default? | Confirm flag |
|---|---|---|---|
| `script/PreflightOptionRfqEntryPoints.s.sol` | Read-only bytecode-scan selector probe for `applyRfqTrade` / `executeRfqTrade` / `applyTrade` / `executeTrade`. | ✅ read-only, no broadcast path | n/a |
| `script/SmokeOptionRfqV2Fees.s.sol` | `view`-only RFQ fee preflight; calls `FeesManagerV2.quoteFees(...)` for both maker + taker legs; asserts FlowKind.RFQ + Tier4/Tier2 amounts. | ✅ `view`-only | n/a |
| `script/SmokeOptionRfqV2FeesExecute.s.sol` | Execute scaffold — refuses without `SMOKE_OPTION_RFQ_V2_FEES_EXECUTE_CONFIRM=true`; even when confirmed, only emits EIP-712 digest. Does NOT broadcast. | ✅ guarded | `SMOKE_OPTION_RFQ_V2_FEES_EXECUTE_CONFIRM` |
| `script/DeployMarginEngineV2.s.sol` | Re-used; deploys new MarginEngine (V2G-O code). | ✅ guarded | `DEPLOY_MARGIN_ENGINE_V2_CONFIRM` |
| `script/DeployOptionMatchingEngine.s.sol` | Re-used; first-deploy OptionMatchingEngine (V2G-O code). | ✅ gated by `--broadcast` | n/a (gated at the forge CLI) |
| `script/RewireMarginEngineV2.s.sol` | Re-used; repoints dependents to new MarginEngine. | ✅ guarded | `REWIRE_MARGIN_ENGINE_V2_CONFIRM` |
| `script/WireFeesManagerV2Option.s.sol` | Re-used; registers new MarginEngine as FM-V2 fee consumer. | ✅ guarded | `WIRE_FEES_MANAGER_V2_OPTION_CONFIRM` |
| `script/ConfigureMarkets.s.sol` | Re-used; activates OPTION product series. | ✅ guarded | per-axis confirm flags |

### 3.2 Solidity tests

| Test | Asserts |
|---|---|
| `test/unit/scripts/PreflightOptionRfqEntryPoints.t.sol` (4 tests) | preflight against zero-address ⇒ NotConfigured, EOA ⇒ TargetHasNoCode, V2G-O stub ⇒ Exposed (all 4 selectors), legacy stub ⇒ ORDERBOOK Exposed + RFQ NotExposed. |
| V2G-O tests in `test/unit/margin/MarginEngine.t.sol` (6 tests) | ORDERBOOK bytecode-equivalence, RFQ Tier 0 == ORDERBOOK, `FeeChargedV2.flowKind=1` decode, Tier 4 maker rebate preserved through MarginEngine, Tier 2 taker 94 ppm, `onlyMatchingEngine` gate on RFQ entry. |
| V2G-N tests in `test/fees/FeesManagerV2.t.sol` (8 tests) | OPTION RFQ taker table walk (Tier 0–4), Tier 4 maker rebate preservation, 100% RFQ discount edge, Tier 0 RFQ == ORDERBOOK, PERP RFQ unchanged, ORDERBOOK unchanged per tier, RFQ ignores negative ppm, setter overflow refusal. |
| V2G-Q + V2G-R2 tier/root/admin tests | 52 tests covering tier matrix, OR-logic, validity windows, replay/upgrade/downgrade, setter boundaries, event emission. |

### 3.3 Backend RFQ operator path (V2G-P1 + V2G-PX)

| Surface | Status |
|---|---|
| `OPTION_RFQ_TRADE_TYPE` const | ✅ live (V2G-M2 picked up V2G-P0 + V2G-S surface) |
| `option_rfq_trade_typehash()` returns `0x6c660d97…f98a123b` | ✅ pinned by backend test against the on-chain `RFQ_TRADE_TYPEHASH` constant |
| `option_rfq_trade_digest{,_bytes}` | ✅ |
| `encode_option_execute_rfq_trade_calldata` | ✅ |
| `option_execute_rfq_trade_selector()` = `0xb52ce6f5` | ✅ pinned |
| `OptionRfqOperatorPacket` / `build_option_rfq_operator_packet` (V2G-P1) | ✅ offline-ready in `target/` |
| `require_option_rfq_broadcast_confirm(env)` — accepts only literal `"true"` | ✅ |
| `OPTION_RFQ_OPERATOR_BROADCAST_CONFIRM_ENV` const | ✅ |
| `payload_summary` redaction (never contains keys / sigs) | ✅ pinned by `packet_summary_does_not_expose_private_key_or_signature` |

V2G-PX adds an operator-walkthrough doc on top of the V2G-P1
primitives: see `OPTION_RFQ_OPERATOR_PACKET_V2G_PX.md` for the
step-by-step "build digest → sign offline → re-build with sigs →
broadcast via executor" sequence.

### 3.4 Frontend admin readiness display

`deopt-v2-frontend/src/app/admin/production-readiness-section.tsx`
already surfaces an **OPTION RFQ readiness** card (V2G-U):

- RFQ fee math: `live` (V2G-N math is in deployed FM-V2)
- RFQ flow wiring (Solidity): `code-ready` (out/ bytecode)
- OptionMatchingEngine (live): `not deployed`
- Backend RFQ signing surface: `code-ready` (target/ binary)
- Operator preflight script: `code-ready`
- Deploy / rewire status: `pending V2G-P operator window`

The card is **read-only**: no wallet hooks, no write affordances,
no signing buttons. Operators who want to drive the V2G-P
broadcast must do it via the V2G-PX runbooks + shell, not the UI.

---

## 4. What is NOT ready (V2G-P broadcast window dependencies)

| Item | Status |
|---|---|
| Operator confirmation of broadcast window date | open |
| Governance multisig approval to broadcast (per V2G-Y matrix — recipient/setter rotation is timelock-72h, but a fresh-deploy + rewire of dependent contracts the operator already owns is timelock-24h or operator-direct depending on governance posture; needs explicit policy call) | open |
| Tier 4 maker EOA (V2G-D2 registry, `0x290bd12c…9274`) funded for the smoke trade | TBD operator-side |
| Tier 2 taker EOA (`0x77ca9dd6…0020`) funded for the smoke trade | TBD operator-side |
| Hardware-wallet (or signing CLI) flow rehearsed by the operator | TBD operator-side |
| Rollback rehearsal on a fork | recommended; not blocking |
| First RFQ trade target premium (gas + amount budget) | per the smoke runbook §3 |

None of these can be closed offline by Claude — they are operator
decisions / external setups.

---

## 5. Manifest + env update plan

### 5.1 Manifest delta (post-V2G-P close)

Append to `deopt-v2-sol/deployments/base-sepolia.manifest.draft.json`:

```json
{
  "v2g_p_broadcast_result": {
    "broadcast_date": "<UTC>",
    "new_margin_engine_v2go": "<addr>",
    "new_option_matching_engine_v2go": "<addr>",
    "first_rfq_tx_hash": "<hash>",
    "first_rfq_block": <n>,
    "rfq_trade_typehash": "0x6c660d979559d8526032a642d665ecefe15ca18cf062c24b6cd36058f98a123b",
    "execute_rfq_trade_selector": "0xb52ce6f5",
    "apply_rfq_trade_selector": "0x1ccdd23f",
    "smoke_maker_address": "0x290bd12c93e467bf51c51f5273d35bddb19e9274",
    "smoke_taker_address": "0x77ca9dd6ccce2d692fb23877a2db7178807b0020"
  },
  "contracts": {
    "MarginEngine": "<new addr>",
    "OptionMatchingEngine": "<new addr>"
  },
  "feesManagerV2WiredToMarginEngine": "<new MarginEngine addr>",
  "feesManagerV2EnabledOnMarginEngine": "<new MarginEngine addr>"
}
```

Mark the OLD MarginEngine (`0x287Cef…48Cc`) and the legacy
MarginEngine (`0x6c5665…5b5f8`) as `observability_only` in the
manifest.

### 5.2 Backend env patch (post-V2G-P close)

`deopt-v2-backend/.env.broadcast.v2e_g.local` (operator-only;
NEVER edited by Claude):

```
# === V2G-P pickup (post-broadcast) ===
MARGIN_ENGINE=<new MarginEngine addr>
OPTION_MATCHING_ENGINE_ADDRESS=<new OptionMatchingEngine addr>

# OLD engines retained for observability classifier only.
MARGIN_ENGINE_OLD=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
```

The operator applies this patch + restarts the backend so the
indexer classifier knows the new vs old engine pair. The V2G-G
observability surface already maps `consumer="new"` / `"old"` /
`"unknown"` against these env vars.

### 5.3 Frontend env

No frontend env change. The static facts in
`src/app/admin/production-readiness-section.tsx::STATIC_FACTS`
should be updated post-V2G-P to swap the placeholder
`optionMatchingEngine: null` for the real address, and add the
new MarginEngine to the live-state surface. That is a
follow-up frontend PR, not a runtime requirement.

---

## 6. Rollback plan

The deploy-rewire runbook (V2G-PX-Deploy) §5 contains the
per-step rollback. Highlights:

- **Pre-broadcast:** none required — no state changed.
- **After Steps 1–2 (deploys):** orphan contracts sit idle; take no action.
- **After Step 3 (rewire):** re-broadcast `RewireMarginEngineV2` with `OLD ↔ NEW` swapped. Reverts cleanly via the V2D-L preflight snapshot.
- **After Step 4 (FM-V2 wire):** `setFeeConsumer(new MarginEngine, false)` + revert `useFeesManagerV2` toggle.
- **After Step 5 (series activation):** `ConfigureMarkets` deactivate.
- **After Step 7 (first RFQ trade):** irreversible on-chain; emergency stop = `pauseTrading` on both engines.

Emergency stop:

```bash
cast send $NEW_OPTION_MATCHING_ENGINE 'pause()' --private-key $GUARDIAN_PK
cast send $NEW_MARGIN_ENGINE 'pauseTrading()' --private-key $GUARDIAN_PK
```

---

## 7. Acceptance criteria for V2G-PX close

- [x] Live RFQ gap re-confirmed (§1).
- [x] Strategy A re-confirmed (§2).
- [x] All Solidity scripts safe-by-default + tested (§3.1, §3.2).
- [x] Backend RFQ operator packet path ready offline (§3.3).
- [x] Frontend admin readiness card live (§3.4).
- [x] Manifest + env update plan documented (§5).
- [x] Rollback path explicit (§6).
- [ ] V2G-P broadcast window scheduled (open).
- [ ] First live RFQ trade landed (open — requires operator session).

The first six rows are closed offline by this V2G-PX pack. The
last two are the explicit operator-side gate that V2G-PX never
crosses.

---

## 8. Hard-gate compliance

V2G-PX broadcasts nothing. Every `cast send` / `forge script
--broadcast` example assumes the human operator at the timelock /
multisig surface. No live state changed by reading this doc.

---

## 9. Cross-links

- V2G-N math: `OPTION_RFQ_FEE_DISCOUNTS_V2G_N.md`
- V2G-O Solidity wiring: `OPTION_RFQ_FLOW_WIRING_V2G_O.md`
- V2G-P0 audit + strategy: `OPTION_RFQ_LIVE_DEPLOYMENT_PREFLIGHT_V2G_P0.md`
- V2G-P1 operator packet primitives: `OPTION_RFQ_OPERATOR_PACKET_V2G_P1.md`
- V2G-PX deploy/rewire runbook: `deopt-v2-sol/docs/OPTION_RFQ_DEPLOY_REWIRE_RUNBOOK_V2G_PX.md`
- V2G-PX smoke runbook: `deopt-v2-sol/docs/OPTION_RFQ_SMOKE_RUNBOOK_V2G_PX.md`
- V2G-PX operator packet walkthrough: `OPTION_RFQ_OPERATOR_PACKET_V2G_PX.md`
- V2G-Y governance matrix (broadcast authority): `GOVERNANCE_ADMIN_SAFETY_MATRIX_V2G_Y.md`
- V2G-T canonical fee audit pack: `DEOPT_V2_CANONICAL_FEE_AUDIT_PACK_V2G_T.md`
