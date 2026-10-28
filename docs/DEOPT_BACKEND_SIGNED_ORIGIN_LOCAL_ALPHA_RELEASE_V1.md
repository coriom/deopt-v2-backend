# DeOpt wallet-level perps alpha: signed-origin persistence

## Decision boundary

The backend now authenticates wrapped `executeTradeFromIntents` calls with an RPC `callTracer` trace, verifies the EIP-712 signers against each D6 fill and D4 wallet settlement, and commits each fill, D2 fee, signed digests and trace-call digest in one PostgreSQL transaction. It stores no raw signatures or traces. The on-chain economic identity remains the wallet; the signed `subaccountId` is order-origin metadata and never becomes an independent settlement account.

Local functional alpha is bounded to wallet-level, read-only reconciliation and synthetic matching on an isolated fork. Real trading, public broadcasting and C02 execution remain unauthorized. The API reports `requires_fresh_chain_recheck` for ordinary historical receipts and `reconciliation_required` for detected reorg or evidence conflicts; it never claims continuous canonicality from a one-time ingestion.

## Evidence and failure handling

- Migration `0071_canonical_v2_fill_provenance.sql` adds an audit row keyed by chain, transaction hash and D6 trade-log index, with a foreign key to the fee row. Existing records stay `legacy_unverified` until authenticated evidence is actually ingested.
- Each signed row records the canonical D6 emitter, block number/hash, buyer and seller wallets, D2 fee-log index, both verified intent digests, and the digest of the nested D6 calldata. This binds the persisted proof to an exact partial fill without storing signatures.
- A wrapper must have one successful D6 call per D6 intent event, in the source-guaranteed call/log order. Each D6 call must contain the expected D4 trade call; the independently decoded signed fill must agree with the D4/D6 wallet event. Ambiguous or malformed trace shapes reject the whole batch.
- An RPC `-32601` trace-method response preserves independently authenticated wallet-level D4/D6/D2 fees with `wallet_only_trace_unavailable`; it does **not** assert signed-origin provenance. A malformed trace is a hard ingestion error.
- If a later provider supplies a trace for a previously wallet-only row, the ingestion path refuses an automatic status upgrade and requires an explicit reconciliation of that historical row.
- Duplicate receipt delivery is idempotent. Conflicting persisted evidence marks the transaction `evidence_conflict` after rolling back the attempted batch. A disappeared or moved receipt marks existing provenance `reorg_divergent`. Economic records remain as audit history; neither condition is automatically reversed or re-promoted.
- The read-only wallet API exposes `signed_origin_provenance` and `canonicality_status`. Detected divergent rows suppress signed subaccount labels and require manual reconciliation. The existing frontend My Fees view continued to display wallet-level fees and the unresolved subaccount notice without source changes.

## Fresh local acceptance

One new **disposable Anvil fork** transaction (`0xa264eae61ab27853c792d84e0e007a93757a655077ea51cc864450e164446082`) emitted two distinct D6 partial-fill events and two D2 fee events in block hash `0x9a224863bbd02ecea1f4418440c4c99a6419575aa0e0c2680e0c5fd039434326`. The same signed maker intent was filled for 1 and then 2 units against two signed takers. Independent fee expectations were 474,660 and 949,320 native six-decimal mUSDC units; the maker fee was zero. The fork-only setup locally executed C02, configured its market and maintenance for simulation, and never sent a public transaction.

The real receipt passed signed trace verification, PostgreSQL atomic ingestion, duplicate replay, two wallet-level buyer rows and one row per seller, fee arithmetic, and API keyset pagination. Negative cases passed for unavailable and malformed traces, call reordering/omission, wrong target and chain, and conflicting persisted origin. A genuine `pg_dump` from the disposable database restored 2 fee rows, 2 signed proofs and 2 provenance rows into a fresh database with the same aggregate digest `31cfb1dc35959024f5562d072236d3c0`. The original database restarted and its API still returned two signed-origin rows. A later intentional Anvil reorg invalidated that local receipt; the backend retained both rows but marked them `reorg_divergent`, and the API returned `reconciliation_required` with no signed subaccount labels.

Clean Chromium against the unchanged local frontend and live local backend passed: 2 buyer fills, one fill per seller, three backend fee-history requests, and zero client exceptions. The focused frontend My Fees suite passed 2/2 tests. The browser fixture now waits for React wallet hydration before clicking and uses DOM-ready navigation because external widgets can delay the full browser `load` event. No production frontend file or deployment changed.

The full Rust library suite passed **1,881/1,881** tests on a serial, loopback-permitted run. The repository-wide `cargo fmt --check` remains failing on existing broad formatting debt; the touched integration test passes targeted `rustfmt --check`. The strict Clippy gate remains failing with 196 existing diagnostics; no lint suppression was introduced. See the machine-readable manifest for exact commands and remaining build/runtime results.

A fresh local Docker release image built successfully. Its non-root, read-only runtime connected to disposable PostgreSQL, returned HTTP 200 for `/health` and `/ready`, and served the reorg-safe API response (`reconciliation_required`, two retained rows, no signed subaccount labels, trading disabled). Its startup log reported execution, funding and liquidation workers disabled and a noop on-chain publisher. The test container was stopped after acceptance; no image was pushed to a registry.

Interrupted/failed attempts are preserved as evidence: Docker's published PostgreSQL loopback port was unreachable from the host, so tests moved to a fresh `/tmp` PostgreSQL instance; the first reorg test exposed a real `null`-receipt handling defect, which was fixed and passed on retry; the first connected browser attempt clicked before React hydration, which was corrected in the test fixture; and a final sandboxed parallel Rust run had two loopback-bind `EPERM` failures plus one shared-environment race. These are not counted as passing tests. The loopback-permitted serial Rust run passed all 1,881 tests on the corrected source.

## Deployment handoff

The backend must remain local for this milestone. A future persistent host needs a non-root Docker runtime, durable PostgreSQL with tested backup/restore, monitored health and readiness, bounded read-only Base Sepolia RPC, and a trusted provider supporting `debug_traceTransaction` with `callTracer` for authenticated wrapped-fill provenance. A basic provider that returns method-not-found can support wallet-level fee history but cannot claim signed-origin linkage for wrappers. Trace provenance and the source-backed call/log ordering are a hard prerequisite for an alpha that advertises signed-origin proof.

Execution, matching workers, funding/liquidation workers, public broadcaster and trading remain disabled by default. Hosted backup, monitoring, authenticated administrative access, provider failover and remote runtime isolation still require separate evidence before deployment. A source-backed C02 handoff can say this milestone did **not** deploy a remote Rust backend; it cannot prove that no earlier remote backend or worker exists. C02 reviewers still need an operator or provider inventory of active Rust/relayer/keeper instances and their effective non-secret execution flags.

## Reproduction and privacy

Run the existing `tests/fixtures/anvil_real_multifill_setup.py` and `anvil_real_multifill.cjs` only against loopback Anvil chain 84532, then run the ignored integration tests in `tests/anvil_real_multifill_fee_history.rs` against disposable PostgreSQL. Run the reorg test last because it intentionally invalidates the local transaction. The receipt/trace bundle, database dump and generated wallets remain under `/tmp` and are not committed. No public wallet key, signature or secret is stored in the new provenance table or evidence.
