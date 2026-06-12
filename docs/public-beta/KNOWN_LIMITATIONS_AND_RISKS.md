# DeOpt V2 — Known Limitations and Risks (Public Beta)

> **Testnet only. No real funds. Unaudited. Experimental. Not mainnet-ready.** Read this before doing anything else.

This document lists the known limitations and risks of the DeOpt V2 Base Sepolia testnet beta. It is intentionally exhaustive — we'd rather over-disclose than have a tester discover a surprise.

---

## 1. Audit and security status

* **Unaudited.** No external audit firm has reviewed the DeOpt V2 contracts, backend, or frontend.
* **No bug bounty (yet).** Please report bugs via the feedback channels in [FEEDBACK_AND_BUG_REPORTING.md](./FEEDBACK_AND_BUG_REPORTING.md). A formal bug-bounty program may be launched later; if and when it is, the rules and scope will be published explicitly.
* **No formal verification.** No portions of the protocol are formally verified.
* **No security review packet has been published yet.** The team plans to publish one (frozen ABI, threat model, invariants, known issues) before any audit engagement; see `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`.

---

## 2. Network and chain

* **Testnet only.** Base Sepolia (chain id `84532`). The protocol is NOT deployed to Base mainnet (`8453`). Do not point your wallet at mainnet RPC URLs.
* **No real funds.** All tokens are testnet mocks. mUSDC (`0x6eAe407f…`) has zero real-world value.
* **Sepolia uptime.** Base Sepolia itself may be unstable from time to time. We have no control over the Sepolia network.

---

## 3. Oracle

* **Mock oracle.** The oracle behind series #0 is a `MockPriceSource` contract owned by the operator. It does not consume Chainlink, Pyth, or any production feed.
* **Tight maxDelay.** The OracleRouter is configured with `maxDelay = 60 s` for the canonical feed. If the operator does not push a fresh price before a trade, `getPriceSafe` returns `(0, 0, false)` and the trade reverts.
* **Operator-side refresh.** Refreshes happen out of band. If you see "stale oracle" warnings, wait for the next refresh, or ping the operator in the feedback channel.
* **No multi-source aggregation.** Production-grade Chainlink-style aggregation is not running; the system uses a primary + secondary mock pair only.

---

## 4. Market making

* **No production market maker.** Pricing for the canonical series is illustrative; quotes come from a mock pricing function tied to the mock oracle.
* **One canonical series.** Series #0 (call on `0x4DeE…`, strike `$3000`, expiry ~`2030-01-01`) is the demonstrated series. Other series may exist but are not actively quoted.
* **Counterparty pairing.** During the beta, you may need to test both buyer and seller sides yourself, or coordinate with another tester. The team may run a basic market-making bot opportunistically; expect gaps.

---

## 5. Backend

* **Local-only backend.** No shared cluster, no high-availability deployment.
* **DB may reset.** The Postgres backend may be wiped or migrated between iterations; intent history is not guaranteed to persist.
* **Indexer lag.** The event indexer may fall behind during heavy network activity. The lifecycle endpoint will catch up eventually, but UI status may flicker.
* **Worker re-runs.** Confirmation and reconciliation workers tick periodically; status changes can lag by up to ~15 s.
* **No high-availability signer.** The backend executor uses a single key in `.env` for the testnet. No AWS / KMS / HSM / Safe-multisig flow is active.

---

## 6. Frontend

* **UX may have bugs.** The UI is in active development. Some flows may not handle edge cases gracefully (e.g., wallet disconnection mid-signing).
* **Wrong-network detection** may be missing or partial on some pages. If something looks off, double-check your wallet is on Base Sepolia.
* **Testnet banner.** A "Testnet — Unaudited" banner should be visible on every page; if you don't see it, that's a bug — please report it.

---

## 7. Smart contracts

* **Address drift.** The protocol underwent a `MatchingEngine` retarget on `2026-06-12`. The canonical pair is now `0x5a5EBF9A…` (matching engine) ↔ `0x506cD65a…` (margin engine). A legacy stale matching engine (`0xf2D1D85…`) still exists on chain but is not part of the canonical flow.
* **Mock collateral token.** mUSDC is owner-mintable. If the operator key were compromised, the token supply could be inflated. This is fine for testnet but obviously not for production.
* **No emergency pause UI.** The contracts have `pause()` selectors; there is no public emergency-pause UI. If you observe an exploit-in-progress on testnet, report it immediately in the feedback channel.

---

## 8. Signature verification

* **Backend signature verification is DISABLED in the public beta.** `SIGNATURE_VERIFICATION_MODE=disabled` in the backend `.env`. This means the backend accepts any well-formed signature blob without verifying it; the actual on-chain `executeTrade` call still verifies signatures via ecrecover, so a bad signature reverts on chain — but it costs gas.
* **EIP-712 domain.** Signatures must be against the canonical matching engine `0x5a5EBF9A…`. Signing against the stale `0xf2D1D85…` will produce a signature that ecrecovers to the wrong address inside the canonical engine and the trade will revert.

---

## 9. Address and API churn

* **Addresses subject to change.** The team may redeploy or re-wire contracts without notice.
* **APIs subject to change.** The backend API surface is frozen at `v2-product-freeze-rc1` for the ABI, but the HTTP routes themselves may be revised. Always check the OpenAPI spec at `deopt-v2-backend/docs/openapi/trading-api.openapi.json` for the current shape.

---

## 10. Uptime and SLA

* **No uptime guarantee.** The beta backend may go down for hours at a time during maintenance, debugging, or chain instability.
* **No SLA.** There is no service-level agreement. There is no support contract.
* **Best-effort feedback response.** The team will read feedback and address bugs as time permits.

---

## 11. Regulatory posture

* **Not a regulated venue.** DeOpt V2 testnet is not a regulated trading venue. No KYC/AML/MiFID compliance is provided or claimed.
* **No solicitation.** Nothing in this documentation pack constitutes a solicitation, offer, or recommendation to buy or sell any financial instrument.
* **Not financial advice.** Nothing here is financial, legal, accounting, or investment advice.
* **Geo restrictions.** If your local laws restrict experimental DeFi participation, do not use the beta. We are not in a position to advise on local law.

---

## 12. Operational risks (testnet-specific)

* **Faucet abuse may rate-limit you.** Excessive faucet requests may be throttled by the faucet provider.
* **Indexer reset may lose lifecycle history.** If the backend DB is reset, your past intents and tx lifecycle history may disappear from the UI. The on-chain truth (Basescan) is unaffected.
* **Operator key rotation.** The team may rotate the executor / owner / deployer keys at any time. Cached front-end state may briefly point at stale addresses.

---

## 13. What we are doing about all of this

* Working through `FRONTEND_TESTNET_LAUNCH_POLISH` (testnet banners, wrong-network handling, copy polish).
* Setting up `COMMUNITY_FEEDBACK_LOOP` (wiring up the placeholder feedback channels).
* Preparing `PRODUCT_FREEZE_AND_SECURITY_REANCHOR` (re-confirming the frozen ABI and assembling the security-review packet).

None of these are audit replacements. Audit comes after, not before, the security-review packet is published and reviewed internally.

---

## 14. Closing reminder

* **Never share your private key or seed phrase.** Not with us, not with anyone.
* **Treat all balances as worthless.** They are mock testnet tokens. Don't trade them off-platform expecting real value.
* **Report bugs.** That's the entire point of a public beta.

---

**End of known limitations and risks.**
