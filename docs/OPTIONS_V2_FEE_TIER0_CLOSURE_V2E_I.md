# Options V2 Fee Tier0 Closure V2E-I

Date: 2026-05-29
Network: Base Sepolia (`chain_id 84532`)
Status: **Options V2 fee path (Tier0 positive, no rebate) production-ready.
Perps phase can start next.**

## Scope

V2E-I closes the options V2 fee programme for the Tier0 positive-fee
launch slice. It is a backend read-only response-shape change: the
already-decoded `basisAmount` field on `FeeChargedV2` / `FeeRebatedV2`
events is now surfaced through both the lifecycle and `/admin/fees/onchain`
payloads. Frontend `LifecycleV2FeeEventCards` (added in V2E-H) already
reads `basis_amount` / `basisAmount` defensively and starts rendering the
value as soon as the new backend ships — no frontend code needed in this
loop.

No broadcast, no submit, no deploy, no Solidity change, no intent or
transaction creation, no live DB mutation, no secret printed.

## What V2E-I Changes

### Backend

`src/fees/onchain_summary.rs`:

- `NormalizedFeeEvent` grew a `basis_amount: Option<String>` field
  (string-preserving so we keep the on-chain uint exactly).
- `normalize_v2_charged` / `normalize_v2_rebated` populate it from
  `decoded["basisAmount"]` (the indexer already decodes it; we only read
  it).
- `normalize_v1_charged` sets it to `None` (V1 `TradingFeeCharged` has no
  basis amount).
- `collect_event_payloads` emits `basis_amount` on `FeeChargedV2` and
  `FeeRebatedV2` JSON entries. V1 entries omit / null it.

`src/api/routes.rs`:

- Integration test `admin_fees_onchain_exposes_v2_charged_and_rebated_totals`
  now also asserts both V2 entries in `events[]` carry the new
  `basis_amount` field (uses the existing decoded `basisAmount = "10000"`
  fixture).

### Tests added (`src/fees/onchain_summary.rs`)

| Test | What it pins |
| --- | --- |
| `v2_charged_normalizes_basis_amount` | normalizer reads `decoded["basisAmount"]` for `FeeChargedV2` |
| `v2_rebated_normalizes_basis_amount` | same for `FeeRebatedV2` |
| `v1_event_has_no_basis_amount` | V1 events stay `None` (no regression for the V1 shape) |
| `collect_event_payloads_surfaces_basis_amount_for_v2` | payload wire-shape: V2 entries carry `basis_amount`, V1 entries null |
| `v2e_g_payloads_expose_basis_amount` | end-to-end regression against the live V2E-G shape: `basis = 50000`, taker = `13`, maker = `3`, mixed `source_priority = "v2"`, lifecycle wrapper carries the field |

Plus the existing
`admin_fees_onchain_exposes_v2_charged_and_rebated_totals` was extended
with two new assertions on the response JSON.

### Frontend

No frontend change in V2E-I. `LifecycleV2FeeEventCards` already reads
`basis_amount` / `basisAmount` from each per-event card (see V2E-H doc
`docs/ADMIN_V2_FEE_OBSERVABILITY_V2E_H.md` §"Per-event V2 cards"). With
this backend change, the admin card stops rendering `n/a` for
`basisAmount` and starts showing the live value (`50000` for the V2E-G
trade).

## V2E-G End-to-End Verification

The new `v2e_g_payloads_expose_basis_amount` test reproduces the V2E-G
trade shape (intent `94897ee5-…`, tx `0xd51ea881…fc72c`, premium 50_000,
Tier0 taker 250 ppm → 13, maker 50 ppm → 3) and asserts:

| Assertion | Value |
| --- | --- |
| `aggregated.event_model` | `"mixed"` |
| `aggregated.source_priority` | `"v2"` |
| `aggregated.charged_total` | `16` |
| `aggregated.rebated_total` | `0` |
| `aggregated.fee_charged_v2_count` | `2` |
| `aggregated.fee_rebated_v2_count` | `0` |
| `aggregated.trading_fee_event_count` | `2` (V1 compat) |
| each `FeeChargedV2` payload `basis_amount` | `"50000"` |
| `lifecycle.observed_total_charged` | `"16"` |
| lifecycle `events[]` entries with `event_name=FeeChargedV2` AND `basis_amount="50000"` | `2` |

Live curl against the running backend was intentionally **not** re-run:
the operator's V2E-G broadcast session is still up on `127.0.0.1:8080`
serving the pre-change binary. Restarting it would interrupt their shell
state (and require re-exposing `EXECUTOR_PRIVATE_KEY`). The new
integration test pins the exact same JSON wire shape, so the live curl is
covered by automation. To re-run against the running backend, the
operator may restart with the rebuilt `./target/debug/deopt-v2-backend`
and re-issue:

```bash
curl -s "http://127.0.0.1:8080/admin/fees/onchain?tx_hash=0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c" \
  -H "X-Admin-Token: $ADMIN_API_TOKEN"
```

Expected response gains `basis_amount: "50000"` on both V2 entries in
`events[]`. All other fields stay identical to the V2E-G result
(`event_model = "mixed"`, `source_priority = "v2"`,
`fee_charged_v2_count = 2`, `observed_total_charged = "16"`, etc. — see
`docs/FEES_MANAGER_V2_TINY_TRADE_BROADCAST_RESULT_V2E_G.md`).

## What Is Validated (Tier0 Positive Option V2 Fees)

- ✅ **FeesManagerV2 deployed** on Base Sepolia
  (`0x00dA0B9876bcBf0c79CB5BcAcfEBAFb8C7Ad774f`).
- ✅ **Wired** into NEW MarginEngine (`setFeesManagerV2` ran in V2E-C).
- ✅ **Enabled** on NEW MarginEngine (`setUseFeesManagerV2(true)` ran in
  V2E-E; tx `0x10c1acff…1a142d`).
- ✅ **Backend indexer emitter** configured
  (`OPTION_EVENT_INDEXER_FEES_MANAGER_V2_ADDRESS = 0x00dA0B…774f`,
  cutover in V2E-D, broadcast-time verification in V2E-G).
- ✅ **Live option trade succeeded** under V2 — V2E-G,
  tx `0xd51ea881cdbc32fe724034c0f7e25ade7359ea3d5b6cadb17b7c345effefc72c`,
  block `42136440`, `status = 1`, `gasUsed = 803_814`.
- ✅ **`FeeChargedV2` indexed** — 2 events from `0x00dA0B…774f`, totalling
  `16` (taker `13` + maker `3`).
- ✅ **Lifecycle / `/admin/fees/onchain` expose V2 source of truth** —
  `event_model = "mixed"`, `source_priority = "v2"`, V1 compat events
  visible but counted zero against totals.
- ✅ **Tier0 positive option fees validated** — `taker = ceil(50_000 ×
  250 / 1e6) = 13`, `maker = ceil(50_000 × 50 / 1e6) = 3`. Ceiling
  rounding is intentional per `FeesManagerV2.sol:401-413`.
- ✅ **`basis_amount` exposed** — V2E-I closes the last V2E-H gap so admin
  cards stop showing `n/a` for the basis the ppm was applied to.
- ✅ **No double-counting** — mixed model picks V2; V1 compat events stay
  visible but contribute zero to totals.
- ✅ **Indexer + reconciliation + lifecycle reconciled** for V2E-G intent
  (`reconciliation.status = "reconciled"`, `health.stage = "reconciled"`,
  `events.total = 21`).
- ✅ **Read-only**: V2E-I added no write paths, no broadcast paths, no
  intent/transaction creation, no DB mutation. The only allowed writes
  on the V2 path are the existing operator-driven admin contract calls
  (`setUseFeesManagerV2`, `setMerkleRoot`, `fundRebateBudget`,
  `setFeeConsumer`, etc.), none of which were exercised in V2E-I.

## What Is Intentionally Deferred

These belong to the post-Tier0 phases of the V2 fee programme and are
**not** required for the perps integration milestone:

- ⏳ **Rebates / negative maker tiers** — Tier0 has only positive maker
  ppm (`50` for OPTION, `50` for PERP). Negative-maker tiers (`Tier2:
  -10 / Tier3: -25 / Tier4: -50` OPTION; `Tier2: -50 / Tier3: -75 /
  Tier4: -100` PERP) cannot be reached without a tier claim (Merkle
  proof) **and** a funded `rebateBudget(settlementAsset)`. Indexer +
  backend already decode `FeeRebatedV2`, `RebateBudgetSpent`,
  `RebateBudgetFunded`, and admin endpoints aggregate
  `rebated_by_trader` / `observed_total_rebated` — but the live path is
  unexercised.
- ⏳ **Rebate budget funding** —
  `FeesManagerV2.rebateBudget(mUSDC) = 0`; no `fundRebateBudget` call has
  been made. Funding is admin-gated and requires an operator broadcast.
- ⏳ **Merkle tier claims** — `FeesManagerV2.merkleRoot() = bytes32(0)`,
  so `claimTier(...)` reverts `InvalidMerkleProof`. Tier-claim flow
  (`setMerkleRoot(root, validFrom, validUntil)` + per-account
  `claimTier(...)`) is exercised in unit tests only.
- ⏳ **RFQ discounts** — `_setRfqDiscountProfile` is populated per tier
  but the RFQ flow (`flowKind = "rfq"`) has not been wired to call
  `FeesManagerV2.consumeFees(... FlowKind.RFQ ...)` on a live RFQ
  acceptance yet. Frontend renders `flow_kind` already.
- ⏳ **Multi-asset fee campaign** — only `mUSDC` is currently used as a
  settlement asset; `FeesManagerV2.rebateBudget(asset)` is keyed per
  asset to allow per-asset campaigns, but no other asset is configured.
- ⏳ **Perps integration** — `PerpEngine.setFeesManagerV2 +
  setUseFeesManagerV2(true)` is the next launch milestone. Backend
  decoder + indexer + `/admin/fees/onchain` are already product-agnostic
  (`product_kind` is rendered per event); frontend `LifecycleV2FeeEventCards`
  already shows `productKind` per card. **No backend or indexer change
  is required before the perp wire** — the perp programme starts at the
  contract layer.

## Criteria For Moving To Perps

All four criteria below are now satisfied:

1. **Tier0 positive option V2 fee path validated end-to-end** — V2E-G
   broadcast succeeded; lifecycle reconciled; admin endpoint reports
   correct V2 totals.
2. **Backend reports basis amount, fee ppm, product kind, flow kind, and
   per-side amounts** on every V2 event — V2E-I (this task) closes the
   last gap (`basis_amount`).
3. **Admin observability ready** — V2E-H frontend changes consume the
   lifecycle + `/admin/fees/onchain` endpoints and render V2 totals
   per-tx and per-event. The `n/a` basis cell is the only field the
   V2E-H doc flagged as outstanding; V2E-I fixes it.
4. **No write / broadcast paths added** in V2E-I — see the no-write proof
   below; the change is purely a JSON-payload field addition routed
   through existing read-only helpers.

**Recommendation: proceed to perps (V2F).** The next milestone is wiring
`PerpEngine.setFeesManagerV2 + setUseFeesManagerV2(true)` and running a
matching tiny-trade preflight + broadcast against the perp product. No
backend, indexer, or frontend change is required before that contract
wire — the V2 telemetry surface already handles `productKind = "perp"`
events transparently.

## No-Write Proof

V2E-I touches:

- `src/fees/onchain_summary.rs` — adds a struct field, populates it on
  the V2 normalizer functions (which only read `decoded`), and adds the
  field to two `serde_json::json!` payloads inside the read-only
  `collect_event_payloads`. No new fetch, no new RPC call, no new DB
  write, no new broadcast.
- `src/api/routes.rs` — only edits inside `mod tests` (extra assertions
  inside the existing `admin_fees_onchain_exposes_v2_charged_and_rebated_totals`
  case); no production code path changed.
- `docs/OPTIONS_V2_FEE_TIER0_CLOSURE_V2E_I.md` — this file.

Verification commands (from `~/DEOPT/deopt-v2-backend`):

```text
git diff --stat
# only the three files above touched

git diff src/fees/onchain_summary.rs src/api/routes.rs \
  | grep -nE "POST|PUT|PATCH|DELETE|send_raw|sendRawTransaction|/broadcast|/executor|persist_|insert_|update_|delete_|setMerkleRoot|fundRebateBudget|setFeeConsumer"
# no matches — V2E-I introduces no write/broadcast/admin-mutation paths
```

`grep -n` over the actual diff hunks confirms the only added tokens are
field reads (`.basisAmount`, `.basis_amount`), the new `Option<String>`
field declaration, two new `"basis_amount": entry.basis_amount` keys in
`serde_json::json!` macros, the V1 `basis_amount: None` line, and the
test cases. No write-path tokens appear in the production diff.

## Validation Commands Run

```text
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --all-targets --all-features
```

Results recorded in §"Validation Results" below.

## Validation Results

| Command | Result |
| --- | --- |
| `cargo fmt --all` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **passed: 606, failed: 0, ignored: 0** (V2E-G baseline was 601; +5 new V2E-I tests) |
| `cargo build --all-targets --all-features` | clean |

Frontend was **not touched** in V2E-I (V2E-H already accepts both
`basis_amount` and `basisAmount`); frontend lint/typecheck/build were
therefore not re-run.

## Files Changed

- `src/fees/onchain_summary.rs` — `basis_amount` field + normalizers +
  payload + 5 new tests.
- `src/api/routes.rs` — extra `basis_amount` assertion inside the
  existing `admin_fees_onchain_exposes_v2_charged_and_rebated_totals`
  test (no production code touched).
- `docs/OPTIONS_V2_FEE_TIER0_CLOSURE_V2E_I.md` — this closure note.

## Final Recommendation

**Proceed to perps (V2F).** All Tier0-option V2 fee acceptance criteria
are green; the basis-amount admin telemetry gap is closed; the V2 wire
shape is product-agnostic and already renders `productKind = "perp"`
without further change.
