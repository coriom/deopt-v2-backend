# DeOpt local Anvil multi-fill acceptance — 2026-10-28

## Result and scope

One **local Anvil** transaction (`0x6384c5fe43fbae6d3d12065199dc1edda82325328b69d225b83a1a068fb19fa3`) emitted two D6 `TradeExecutedFromIntents` events and two D2 `FeeChargedV2` events. Its local block hash was `0xb64dd58c311afb38ec0044b94140278286469ae4c1c1c9bd9dee31a48cc91228`. The same signed maker intent filled 100,000,000 then 200,000,000 units of 1e8 size; D6 `intentFilled` read back 300,000,000. Two distinct takers incurred 474,660 and 949,320 native six-decimal mUSDC units at 200 ppm. The maker incurred zero. The trace showed two successful nested D6 calls, each with two D4→D2 calls. Rust verified both EIP-712 signatures, exact nested calldata, D6 logs, trade sizes and roles, canonical local receipt/block identity, and the fee arithmetic.

The backend's existing receipt reconciliation inserted both fills and fees in one PostgreSQL transaction. A repeated invocation inserted nothing. A PostgreSQL container restart preserved the rows; another replay inserted nothing. The actual read-only HTTP API served two maker rows and one row per taker. A clean local Chromium session showed the same rows in My Fees, including wallet-only attribution and the explicit subaccount limitation; it observed no uncaught client exception. The full Rust library suite passed 1,881 tests. None of this is a public Base Sepolia trade or C02 authorization.

The unchanged frontend's focused Playwright suite also passed 2/2 tests, covering wallet disconnect/reload, wrong-wallet data rejection and backend-unavailable display. The backend Docker image reused from the prior accepted source build started non-root with a read-only filesystem, returned HTTP 200 for both `/health` and `/ready`, and logged execution and funding/liquidation workers disabled. A fresh image build was unnecessary because this milestone changed only test/evidence files.

## Reproduction boundary

All fixture scripts hardcode `http://127.0.0.1:18545`, require `web3_clientVersion` to identify Anvil and require chain ID 84532. Start from a **fresh isolated fork** at a block after the five D2 executions and while the reviewed C02 operation is executable but not expired. The setup verifies the exact committed C02 Builder SHA-256, executes C02 *only on the fork*, deploys a disposable clearing contract and configures D4, D6, D2 and Vault **only on the fork**. The transaction fixture adds two fresh mock oracle sources on the fork because the fork's original oracle samples become stale during a long local test; it does not relax source checks on the public chain. The wrapper is a local test contract with no deployment path.

```bash
# Use an operator-controlled read-only Base Sepolia RPC for the fork source.
anvil --fork-url "$BASE_SEPOLIA_READ_RPC" --chain-id 84532 --port 18545 --silent
docker run --rm -d --name deopt-anvil-fees-pg \
  -p 127.0.0.1:15439:5432 \
  -e POSTGRES_HOST_AUTH_METHOD=trust -e POSTGRES_DB=deopt_anvil_fees postgres:16-alpine
cd /home/corio/DEOPT/deopt-v2-backend
forge build --root "$PWD" --contracts tests/fixtures \
  --out /tmp/deopt_anvil_forge_out --cache-path /tmp/deopt_anvil_forge_cache
python3 tests/fixtures/anvil_real_multifill_setup.py
node tests/fixtures/anvil_real_multifill.cjs
DEOPT_REQUIRE_NEW_INGEST=1 \
DEOPT_ANVIL_MULTIFILL_FIXTURE=/tmp/deopt_anvil_multifill_actual.json \
DEOPT_ANVIL_MULTIFILL_PG_URL=postgres://postgres@127.0.0.1:15439/deopt_anvil_fees \
  cargo test --test anvil_real_multifill_fee_history \
  real_anvil_multifill_receipt_to_postgres_and_api -- --ignored --nocapture
```

The JSON receipt/trace bundle remains under `/tmp`; it is not committed. The two integration tests are deliberately ignored by the default suite so a missing Anvil or PostgreSQL cannot be reported as a passing integration test. The reorg test is destructive **only to the disposable Anvil snapshot** and must run after the successful ingestion and browser tests:

```bash
DEOPT_ANVIL_REORG_CHECK=1 cargo test --test anvil_real_multifill_fee_history \
  real_anvil_reorg_fails_closed_without_erasing_committed_accounting \
  -- --ignored --nocapture
```

The local frontend test uses `tests/fixtures/verify_anvil_fee_browser.cjs`; it requires the unchanged frontend at `127.0.0.1:3001` and the backend at `127.0.0.1:4010`, with `NEXT_PUBLIC_TRADING_API_BASE_URL=http://127.0.0.1:4010`. Both servers must have trading/broadcast flags explicitly false. It uses a read-only injected wallet provider and no live signing key.

## Evidence limits

- **Settlement identity:** D6 and D4 settle at wallet level. The signed `subaccountId` is order-origin metadata, not proof of a separate on-chain economic owner. The backend persists canonical fees at wallet level with `subaccount_id = null`.
- **Trace use:** The local acceptance harness authenticated `callTracer` against the real local transaction and used nested calldata to verify both signatures. The production-style backend persistence path still uses D4/D6 event pairs for wrapped wallet-level accounting; it does not persist a trace-derived subaccount proof. If a future workflow requires signed origin for each wrapped fill in the stored record, add a reviewed trace-backed ingestion path. A missing or ambiguous trace must never be represented as subaccount settlement proof.
- **Reorg:** After `evm_revert`, the backend refused to reconcile the disappeared receipt. It deliberately retained the already committed rows with `requires_fresh_chain_recheck`; operator reconciliation remains necessary. It did not silently reverse settled accounting.
- **Lint:** The repository-wide strict Clippy gate remains separate from this acceptance. No blanket lint suppression was introduced. The new integration file passes targeted rustfmt; existing source-wide formatting debt is unchanged.
- **Production frontend:** No frontend file or Vercel deployment was changed. This milestone verified the local browser using real test rows, not the current hosted site.

## C02 operational handoff

The known Rust backend executable is `deopt-v2-backend`; local HTTP, optional indexer, matching, execution, funding and liquidation workers are controlled by runtime flags. This milestone started only loopback test processes. The Docker smoke used a non-root, read-only container with `EXECUTION_ENABLED=false`, `EXECUTOR_REAL_BROADCAST_ENABLED=false`, and `PERPS_PUBLIC_TRADING_ENABLED=false`; funding/liquidation workers were disabled. Existing Railway references in historical docs describe a failed deployment attempt and are **historical, unverified as to current service state**. No remote Rust backend, relayer or keeper state was independently verified here. The separate C02 reviewer still needs one timestamped provider inventory or operator attestation identifying every current backend/worker host, effective chain/contract targets, and non-secret transaction-submission flags. Unknown remote services remain UNKNOWN, and C02 execution remains blocked.

## Next local action

If separate on-chain subaccount settlement is required, define a protocol-level authenticated economic identity and review it independently. For the wallet-level alpha, keep broadcaster off and address repository-wide strict lint debt plus hosted-only monitoring/backup readiness before any remote deployment decision. Public trading additionally requires independent governance, market, migration, maintenance and operational authorization.
