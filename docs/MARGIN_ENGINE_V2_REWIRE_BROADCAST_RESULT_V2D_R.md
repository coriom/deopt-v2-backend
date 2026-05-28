# MarginEngine V2 On-chain Rewire + Backend-Cutover Pointer — V2D-R

Date created: 2026-05-27 (backstop record, no Solidity changes here)
Status: rewire complete on chain; backend cutover completed in V2D-S.

## Source-of-truth note

This file was not present in the backend repo before V2D-S. It is created
now as a backend-side companion to the on-chain rewire work so that
V2D-S has a stable predecessor doc to link to. The authoritative
record of the on-chain rewire (deploy + role-grants + per-contract
`setMarginEngine` calls) lives in the Solidity / ops repo, not here.

## On-chain rewire summary (read from chain, May 2026)

- OLD_MARGIN_ENGINE          `0x6C5665De05e7314cB63cD77F82DFa86508A5b5F8`
- NEW_MARGIN_ENGINE          `0x287Cef479be5889eEfCa847F9e73C860898f48Cc`
- Cutover block              `42073772`
- CollateralVault.marginEngine  → NEW
- RiskModule.marginEngine        → NEW
- MatchingEngine.marginEngine    → NEW
- OptionMatchingEngine.marginEngine → NEW
- InsuranceFund                  → OLD disabled / NEW enabled
- RiskGovernor.marginEngine      → NEW
- NEW.feesManager                = V1 FeesManager `0xaef73F10224712E1312963BE11662061481aA0F0`
- NEW.feesManagerV2              = `0x0000000000000000000000000000000000000000`
- NEW.useFeesManagerV2           = `false`

OLD MarginEngine remains deployed but is no longer wired in upstream
consumers. Historical events emitted by OLD remain on chain and remain
indexed in the backend DB; lifecycle reads continue to surface them
for V1S and earlier trades.

## Backend cutover status

| Item | Status |
| --- | --- |
| Backend `MARGIN_ENGINE` env switched to NEW | ✅ (V2D-S, runtime override) |
| Backend indexer `MARGIN_ENGINE_ADDRESS` switched to NEW | ✅ (V2D-S) |
| `/admin/config` reports NEW as current margin engine | ✅ (V2D-S step 4) |
| Read-only cast checks confirm V2-disabled NEW | ✅ (V2D-S step 5) |
| Historical V1S lifecycle still reconciled | ✅ (V2D-S step 6) |
| Admin events / fees endpoints non-broken | ✅ (V2D-S step 7) |
| FeesManagerV2 enabled | ❌ — intentionally off; out of scope |
| Tiny test trade broadcast | ❌ — separate task; see V2D-S §"Remaining Blocker" |

For the full backend-side cutover evidence and validation run, see
[`MARGIN_ENGINE_V2_BACKEND_CUTOVER_V2D_S.md`](MARGIN_ENGINE_V2_BACKEND_CUTOVER_V2D_S.md).
