# DeOpt D6 intent receipt acceptance

## Decision

Local acceptance is **PARTIAL**. A direct `executeTradeFromIntents` call now has a source-compatible verification and wallet-level D2 fee persistence path. A nested batch with multiple D6 fills is rejected until authenticated call traces bind every individual calldata payload to its log. There is no public trading or backend broadcast authorization.

## Proof boundary

The frozen D6 ABI selector is `0xb69459b0`. D6 emits two EIP-712 intent digests and trade size/price/time; it does not emit wallets or subaccounts. The verifier decodes the transaction input, requires the exact D6 destination and Base Sepolia chain, re-encodes the ABI, verifies both V2 signatures, checks the signed bounds and echoed IDs, then compares both event topics and all event data with the canonical receipt. The block hash is rechecked before PostgreSQL persistence.

Signed `subaccountId` is an **order-origin label**, not a canonical economic subaccount. D4 positions and settlement are keyed by wallet and market. No local SQL row is inferred or created by receipt ingestion. The API therefore exposes `signed_subaccount_id` separately while retaining `subaccount_id: null` and wallet-level fee liability. A future economically separate subaccount requires explicit contract/accounting semantics, authenticated ownership mapping and independently reviewed governance; a UI label alone cannot provide this proof.

The new migration stores intent hashes and signed IDs alongside the canonical fee key `(chain_id, tx_hash, trade_log_index)`. Fill, D2 fee and proof commit in one PostgreSQL transaction. Replays with matching data are idempotent; conflicting block hashes or proof fields fail closed. No signature bytes, raw calldata, wallet keys or secret configuration are persisted in the new table.

## Fresh validation

| Gate | Result | Evidence |
| --- | --- | --- |
| Frozen ABI and event | PASS | Compiled D6 ABI plus selector/topic regression test |
| Rust library | PASS | `cargo test -j 1 --lib -- --test-threads=2`: 1879 pass, 0 fail with local listener permission; sandbox-only run had two `EPERM` failures |
| Disposable PostgreSQL | PASS | One live test with migrations, atomic insert, retry, wallet isolation and reorg divergence |
| Multi-fill D2 reconciliation | FAIL | The direct D6 path accepts one intent event; nested calls need authenticated traces to pair each fill with its D2 charge |
| HTTP on loopback | PARTIAL for full receipt path | `/health` 200; `/ready` 200 with `perps_trading_status=disabled`; synthetic canonical wallet history 200; admin reconciliation without credentials 403. Authenticated reconciliation is closed until D2 S2 proof. |
| Current D2 chain | FAIL for approved policy | Safe block 50887065 has tiers `[50,300],[0,250],[-50,200],[-75,175],[-100,150]` ppm and all five S1 operations queued; final safe block 50900713 at 11:53:31 UTC still has tier 0 `[50,300]` and tier 4 `[-100,150]`, with block hash and tier 0 corroborated by a second RPC |
| Frontend Chromium | FAIL for E2E acceptance | Existing 7-test Perps subaccount suite: 3 pass, 4 fail because it expects a removed account-activity strip. No canonical fee component is mounted. Frontend repository restored clean. |
| Strict Clippy | FAIL | 196 library diagnostics, plus one final compilation summary; baseline 196; no new diagnostic in the intent verifier |
| Anvil V2 lifecycle | PASS, limited scope | Existing local V2 positive lifecycle test passed once against disposable PostgreSQL; it did not exercise an `executeTradeFromIntents` receipt or nested batch |
| Docker image and local runtime | PASS | Exact final-source release image built; localhost `/health` and `/ready` returned 200; non-root user and read-only filesystem; trading and workers disabled |

The public D2 reads are **read only**. At the cited safe blocks, the approved 0/200 ppm PERP policy was not active; `ONCHAIN_D2_STATE_VERIFIED=NO`. The local fee example is explicitly synthetic and cannot be presented as a public-chain fill.

## Remaining integration work

1. Extend reconciliation to wrapper transactions using authenticated nested D6 call traces. Bind each call's two signed hashes to exactly one D6 event and exactly one preceding D2 charge segment, including several fills in one outer transaction. Reject ambiguous traces and duplicates.
2. Add a read-only canonical fee history panel to the frontend in its separately controlled repository. Fetch `/accounts/{wallet}/perps/canonical-fees` without subaccount filtering; show wallet fee, signed order-origin ID and unresolved economic attribution distinctly. Do not mix local projected fills with settled facts. The current Perps page deliberately removed the account activity strip, so the old routing tests require a separate frontend update.
3. After five genuine D2 S2 execution postflights, re-run the pinned policy and receipt proof. Even a verified D2 profile cannot enable trading without independent market, maintenance, migration and operational gates.

No Solidity, cloud or frontend production files were changed. No public transaction was sent.
