# DeOpt V2 — Public Docs Beta Checklist

**Date:** 2026-06-10
**Posture:** docs-only plan. **No source code modified.** Companion to
`PRODUCT_READINESS_ROADMAP.md`. The pack delivered at M-P6 closure.

## 1. Hard rules for every public doc

- **"DeOpt is not yet audited."** Banner appears at the top of every page.
- **"DeOpt is a testnet beta."** Sub-banner explains current state.
- No mainnet deployment timeline. No mainnet contract addresses. No production AWS values. No vendor commitments.
- No production EVM addresses (BACKEND_EXECUTOR / DEPLOYER / production signer).
- No production RPC URL.
- No admin Bearer token / API key in examples.
- No `.env` content beyond Sepolia rehearsal-safe values explicitly marked `<EXAMPLE_REPLACE_ME>`.

## 2. Doc inventory

### 2.1 Root README (operator-public)

`~/DEOPT/README.md` (TBD; not yet present at top level) or
`~/DEOPT/deopt-v2-sol/README.md` + `~/DEOPT/deopt-v2-backend/README.md` +
`~/DEOPT/deopt-v2-frontend/README.md` (each updated for beta posture).

Must include:

- One-line project summary.
- "Not yet audited" banner.
- Testnet-only badge.
- Architecture diagram pointer.
- Links to quickstart, testnet guide, user guide, developer API docs.
- Links to risk disclosures + known limitations.

### 2.2 Quickstart

`docs/QUICKSTART.md` per repo (or top-level `docs/QUICKSTART.md`).

Steps:

1. Prereqs (foundry, rust toolchain, node 22+, docker optional).
2. Clone + install per repo.
3. Run local stack (anvil + backend + frontend).
4. Connect a wallet (anvil[1]); deposit; trade; exercise.
5. Where to go next (links to user guide, MM guide, dev API docs).

### 2.3 Testnet guide

`docs/TESTNET_GUIDE.md` (operator-public).

- Base Sepolia chain id (84532), explorer link.
- Sepolia faucet links.
- DeOpt Sepolia anchor addresses (operator-public; same as listed in `MAINNET_AUDIT_EXT_KICKOFF_FINAL.md §8`).
- How to connect a wallet to Base Sepolia.
- Where to claim test mUSDC.
- How to deposit / trade / exercise / withdraw on Sepolia.
- Troubleshooting (RPC issues, nonce sync, gas).

### 2.4 User guide

`docs/USER_GUIDE.md`.

- What is a European option (concise: strike, expiry, call/put, settlement).
- How DeOpt prices options (oracle-based; no IV surface yet).
- How fees work (`FeesManagerV2` signed-ppm; takerFee + makerRebate DEFERRED at launch).
- Margin model (basic explanation of IM / MM / equity / free collateral).
- Liquidation overview.
- Insurance fund overview.
- Exercise + settlement mechanics.
- Risk disclosures.

### 2.5 Market maker guide

`docs/MARKET_MAKER_GUIDE.md`.

- RFQ flow (taker creates RFQ → makers quote → taker accepts).
- Quote-signing payload format (EIP-712 envelope).
- API endpoints for MM: `GET /options/rfqs`, `POST /options/rfqs/:id/quotes`, etc.
- Slippage + ttl semantics.
- Cancellation semantics.
- Off-chain signing recommendations (key management; no client-side raw key).

### 2.6 Developer API docs

`docs/DEVELOPER_API.md` (or OpenAPI export from backend).

- Auth posture: trading endpoints are user-anchored (EIP-712 signature on every signing-payload); no Bearer for trading.
- Admin endpoints: gated by SSR proxy + OIDC/MFA + Bearer (V2G-W3 closure).
- Per-endpoint: method + path + request schema + response schema + error codes + example curl.
- Pagination + error envelope conventions.
- Rate limits (M-P2 sets; sketch only for now).

### 2.7 Smart contract docs

Already present in `deopt-v2-sol/`: `SPEC.md`, `ARCHITECTURE_MAP.md`,
`INVARIANTS.md`, `PARAMETERS.md`, `ROLE_MATRIX.md`, `TEST_MATRIX.md`,
`DEPLOYMENT_PLAN.md`, `BASE_SEPOLIA_REHEARSAL.md`.

Beta additions:

- `docs/CONTRACTS_PUBLIC_REFERENCE.md` (operator-public; abridged
  re-export of the canonical contract list + Sepolia addresses; placeholders
  for mainnet).
- `docs/ABI_REFERENCE.md` (auto-generated from foundry; link to compiled
  ABIs at the freeze commit).
- `docs/SECURITY_MODEL.md` (operator-public; summarises invariants, role
  model, defence-in-depth, audit status = "not yet audited").

### 2.8 Architecture overview

`docs/ARCHITECTURE_OVERVIEW.md` (operator-public, beta level).

- Layered diagram (sol contracts → backend executor → frontend trading UI).
- Off-chain matching + on-chain execution model.
- EIP-712 signing flow (taker / maker / backend).
- Event indexer + reconciliation.
- Health + observability.
- AWS KMS posture (rehearsal-only at beta).

### 2.9 Risk disclosures

`docs/RISK_DISCLOSURES.md`.

Required disclosures:

- DeOpt is NOT yet audited externally.
- DeOpt is a testnet beta. Test tokens have no real-world value.
- Smart contract risk: unknown bugs may exist.
- Oracle risk: oracle outage may halt settlement.
- L2 sequencer risk: Base sequencer outage may delay or halt operations.
- Liquidation risk: positions can be liquidated.
- Custody risk: testnet only — do not deposit real funds.
- Governance risk: testnet uses operator-controlled multisig; mainnet posture documented in `MAINNET_CUSTODY_POLICY.md` but NOT YET ACTIVE.
- No guarantee of uninterrupted service.
- No commitment to a mainnet timeline.

### 2.10 Unaudited / testnet warning surface

Permanent visual surfaces (M-P3 implements):

- Frontend top banner: "**Unaudited testnet beta**" — sticky, cannot be dismissed.
- README badge.
- Quickstart top notice.
- User guide top notice.
- Developer API top notice.

### 2.11 FAQ

`docs/FAQ.md`.

Sample:

- Why isn't this audited yet?
- When will mainnet launch?
- How do I report a bug?
- Why does my wallet show "wrong network"?
- Why is the Trade button disabled?
- What is RFQ and why use it?
- How is mark price computed?
- How are fees calculated?
- How are options settled?

### 2.12 Troubleshooting

`docs/TROUBLESHOOTING.md`.

Sample:

- Wallet not detected.
- Network mismatch.
- Stuck transaction / nonce drift.
- Stale quote.
- Insufficient collateral / balance.
- Backend offline.
- Sepolia RPC issues.
- Faucet rate limits.

### 2.13 Known limitations

`docs/KNOWN_LIMITATIONS.md`.

Must include:

- Perp markets out of scope at beta.
- Rebates DEFERRED at launch (Cluster 4 Q-CD-11).
- Liquidation surface not yet smoke-tested on Sepolia.
- Backend executor is operator-controlled (V2G-Y migration not yet executed on mainnet).
- AWS KMS posture is rehearsal-only at beta.
- Admin frontend uses sessionStorage admin Bearer (F-H1 closure planned via V2G-W3 SSR proxy; M-P3 prerequisite).
- No mobile UI yet.
- No L2 sequencer outage UI yet beyond the health-endpoint banner.

### 2.14 Bug-report channel

`docs/BUG_REPORTING.md`.

- Email / repo issues / private security disclosure link.
- Severity ladder (Critical / High / Medium / Low / Info; matches
  `MAINNET_AUDIT_EXT_KICKOFF_FINAL.md §14`).
- "Responsible disclosure window: 90 days" if applicable.

## 3. Doc ownership

| Doc | Owner | Reviewer |
|---|---|---|
| README (each repo) | repo owner | docs lead |
| QUICKSTART | docs lead | each repo owner |
| TESTNET_GUIDE | docs lead | backend + frontend |
| USER_GUIDE | docs lead | sol |
| MARKET_MAKER_GUIDE | docs lead | backend |
| DEVELOPER_API | backend | docs lead |
| CONTRACTS_PUBLIC_REFERENCE | sol | docs lead |
| ABI_REFERENCE | sol (auto) | n/a |
| SECURITY_MODEL | docs lead | sol + backend |
| ARCHITECTURE_OVERVIEW | docs lead | sol + backend + frontend |
| RISK_DISCLOSURES | docs lead | legal review (post-beta if applicable) |
| FAQ | docs lead | community |
| TROUBLESHOOTING | docs lead | backend + frontend |
| KNOWN_LIMITATIONS | docs lead | all repo owners |
| BUG_REPORTING | docs lead | security |

## 4. Generated artifacts

- ABI dumps from `forge build`.
- OpenAPI / JSON-schema export from backend (`utoipa` or similar).
- Test-result badges (Sol forge / backend cargo / frontend Playwright).

## 5. Hosting

- Suggestion: `docs/` per repo + a top-level `docs/` aggregator served via Next.js or a static-site generator (no vendor lock-in).
- No third-party tracking / analytics in beta.
- No external CDN for assets beyond Next.js defaults; consider self-hosted.

## 6. Pre-publish gate (operator-side)

Before publishing:

- All "no production EVM / no real AWS / no Bearer / no RPC URL" sweeps pass.
- "Not yet audited" banner present on every page.
- Testnet-only badge present on every page.
- Cross-links resolve.
- ABI references compile-frozen at the M-P6 closure commit.

## 7. Cross-links

- `PRODUCT_READINESS_ROADMAP.md`
- `PRODUCT_GAP_ANALYSIS_SOL_BACKEND_FRONTEND.md`
- `TRADING_INTERFACE_REQUIREMENTS.md`
- `E2E_TRADING_LIFECYCLE_TEST_PLAN.md`
- `NEXT_PRODUCT_MILESTONES.md`
- `~/DEOPT/MAINNET_CUSTODY_POLICY.md`
- `~/DEOPT/deopt-v2-sol/README.md`

**End of public docs beta checklist.**
