# PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_WORKER_V1

Real Base Sepolia (chainId 84532) closed-test broadcast path for the
Perp matching-engine settlement flow. Implemented in
[`src/execution/broadcast_policy.rs`](../src/execution/broadcast_policy.rs).

## Scope

- CLOSED-TEST only. Public Perps remains fail-closed at the route
  boundary (`src/api/routes.rs:3076-3078`).
- No mainnet path. Startup config already refuses mainnet local-signer
  (`src/execution/config.rs:158-162, 218-234`).
- No fabrication of trader fixtures / market registrations / collateral
  positions in this milestone. Those land under
  `PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`.
- No public route added. The worker is a background/tick component
  invoked by the executor runner; no new HTTP surface.

## Runtime lifecycle

```
matched intent (Pending)
  ↓ dry-run preview tick (already exists) — status → DryRun
  ↓ simulation (if enabled) — status → SimulationOk / SimulationFailed
  ↓ BroadcastPolicy::broadcast_intent
    ├── preflight_static
    │   ├── execution_enabled
    │   ├── real_broadcast_enabled
    │   ├── dry_run == false
    │   ├── rpc_url configured
    │   └── signer.address() == executor_from_address
    ├── ensure_no_submitted_transaction (idempotency guard)
    ├── rpc.chain_id() == executor_chain_id
    ├── preflight_pme_state
    │   ├── PME.isExecutor(from) == true
    │   └── PME.paused() == false
    ├── build_execution_transaction_request  (validates
    │   simulation_ok status + calldata_ready)
    ├── rpc.transaction_count(from, pending)  → nonce
    ├── eip1559_transaction_prehash → signer.sign_prehash
    │   → assemble_eip1559_signed_transaction  → raw_hex
    ├── derive_signed_transaction_hash        → tx_hash
    ├── repository.record_submitted_transaction (BEFORE send)
    ├── rpc.send_raw_transaction(raw_hex)
    │   ├── ok  → tx_hash echoed; status → Submitted
    │   └── err
    │       ├── AlreadyKnown / NonceTooLow → fall through to poll
    │       └── Other → repository.mark_intent_failed; return Failed
    ├── poll rpc.transaction_receipt(tx_hash) × 15 × 2s
    │   ├── receipt.status = 1 → mark_intent_confirmed; return Confirmed
    │   ├── receipt.status = 0 → mark_intent_failed; return Failed
    │   └── no receipt within budget → return Submitted (reconcile later)
```

## Fail-closed gate matrix

The following table maps each fail-closed predicate to its enforcing
site and rejection error class:

| Gate | Enforcing site | Rejection |
|---|---|---|
| execution_enabled = false | `preflight_static` | `BroadcastRejected` |
| real_broadcast_enabled = false | `preflight_static` | `BroadcastRejected` |
| dry_run = true | `preflight_static` | `BroadcastRejected` |
| rpc_url not configured | `preflight_static` | `BroadcastRejected` |
| signer.address() ≠ EXECUTOR_FROM_ADDRESS | `preflight_static` | `BroadcastRejected` |
| prior submitted tx_hash present | `ensure_no_submitted_transaction` | `BroadcastRejected` |
| rpc chain_id ≠ EXECUTOR_CHAIN_ID | `broadcast_intent` | `BroadcastRejected` |
| PME.isExecutor(from) = false | `preflight_pme_state` | `BroadcastRejected` |
| PME.paused() = true | `preflight_pme_state` | `BroadcastRejected` |
| simulation_ok status required but missing | `build_execution_transaction_request` | `BroadcastRejected` |
| signatures missing | `build_execution_transaction_request` | `MissingTradeSignatures` |
| receipt.tx_hash ≠ derived envelope hash | `finalize_receipt` | `BroadcastRejected` |
| receipt.status = 0 | `finalize_receipt` (mark_intent_failed) | `Failed` state |
| mainnet local-signer | startup (`validate_signer_backend`) | `Config` |

## Idempotency / crash-safety

Failure boundaries:

1. **Before signing** — no side effect; safe to retry.
2. **After signing, before persistence** — no side effect; safe to
   retry (a new nonce fetch will pick up any prior sends).
3. **After persistence, before RPC send** — the repository row has a
   tx_hash but the chain does not. On restart, the reconciler polls
   `transaction_receipt(tx_hash)` and observes `None`. If enough time
   has elapsed the operator resets the intent manually; automatic
   rebroadcast is refused because `ensure_no_submitted_transaction`
   short-circuits.
4. **After RPC send, before persistence** — persistence lands FIRST by
   design; this window does not exist.
5. **After receipt, before status transition** — reconciler observes
   the receipt again and transitions.

Guarantees:
- No duplicate `eth_sendRawTransaction` for an intent unless the row
  is explicitly reset.
- Deterministic reverts (`receipt.status = 0`) never rebroadcast.
- On-chain intent nonce accounting (`intentFilled[hash]` in
  `PerpMatchingEngine`) is authoritative for economic replay
  protection — the backend broadcast worker does not attempt to
  mirror it.

## Executor nonce policy

`BroadcastPolicy` reads `transaction_count(executor, pending)` once
per intent. Serialization of conflicting sends from the same executor
must be enforced by the caller loop (single-threaded tick, or one
outstanding intent at a time). `SendErrorClass::NonceTooLow` and
`::AlreadyKnown` are treated as idempotent replays and fall through
to receipt polling.

## Signer binding

The worker requires that the runtime signer's address equals
`config.executor_from_address` (`preflight_static`). When
`HybridV2KmsSignerBridge` is used, its `probe()` cross-checks against
`HV2_SIGNER_EXPECTED_ADDRESS`. Combined:

- `signer.address()` must equal `executor_from_address`
- `bridge.expected_signer_address` must equal the RemoteSigner's
  actual signing address (verified at every sign call —
  `IdentityMismatch` returned otherwise)
- Both must equal the on-chain `PME.isExecutor(_) == true` address

## Environment variables (names only)

| Env var | Purpose |
|---|---|
| `EXECUTOR_ENABLED` | Master switch for the executor tick |
| `EXECUTOR_DRY_RUN` | Preview-only mode; MUST be false for real broadcast |
| `EXECUTOR_REAL_BROADCAST_ENABLED` | MUST be true for real broadcast |
| `EXECUTOR_POLL_INTERVAL_MS` | Tick loop cadence |
| `EXECUTOR_MAX_BATCH_SIZE` | Rows per tick |
| `EXECUTOR_CHAIN_ID` | MUST equal 84532 for this milestone |
| `EXECUTOR_FROM_ADDRESS` | Runtime executor EOA (currently the rotated `0x58Ad…52B8`); operator MUST set to match on-chain state |
| `EXECUTOR_PRIVATE_KEY` | LocalDev signer only; refused on mainnet |
| `EXECUTOR_ALLOW_LOCAL_SIGNER` | Explicit opt-in for LocalDev on testnets |
| `EXECUTOR_MAX_GAS_LIMIT` | Envelope gas limit |
| `EXECUTOR_MAX_FEE_PER_GAS_WEI`, `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` | EIP-1559 fee cap |
| `BACKEND_SIGNER_MODE` | `local_dev` \| `remote` |
| `BACKEND_SIGNER_ENDPOINT` | Required if mode=remote |
| `BACKEND_SIGNER_PROVIDER` | `aws_kms` / `gcp_kms` / `turnkey` / `mock` (mainnet REFUSES `mock`) |
| `BACKEND_SIGNER_TIMEOUT_MS` | Per-request signer timeout (100–30 000, default 2500) |
| `RPC_URL` (or `HV2_BROADCAST_RPC_URL` for Hybrid V2 pipe) | Required if real broadcast enabled |
| `PERP_MATCHING_ENGINE_ADDRESS` | Broadcast target contract |
| `PERPS_CLOSED_TEST_ENABLED` | Route-level gate for the signed-intent HTTP path (upstream of broadcast) |
| `PERPS_CLOSED_TEST_ALLOWLIST` | Comma-separated allowlisted callers |

No secret values appear in this document. Operators MUST supply
`EXECUTOR_FROM_ADDRESS` and `HV2_SIGNER_EXPECTED_ADDRESS` at deploy
time. Never commit those values to the repository — they are
deployment-only.

## Closed-test operational posture

- **Public Perps remains OFF.** No changes to the fail-closed route
  gate at `src/api/routes.rs:3076-3078`.
- **Funding remains OFF.** No funding-tick worker changes.
- **Collateral activation unchanged.** No new WETH / cbBTC gating.
- **Base mainnet untouched.** Chain-id preflight refuses non-84532.

## Follow-on work (NOT in this milestone)

1. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`** — deliberately
   fund two closed-test trader wallets with mUSDC collateral via the
   CollateralVault. Track their subaccount balances.
2. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_SMOKE_ORDER_V1`** — pre-sign a
   matched-pair `PerpOrderIntent` bundle against market 1 (ETH-PERP),
   route via the new broadcast worker, verify receipt.
3. **Postgres persistence for `execution_intents`** — the repository
   trait's five new methods currently have fail-closed defaults;
   production wiring must implement them against Postgres. Recommended
   schema addition: `execution_intent_broadcasts` table with columns
   `(intent_id PK, tx_hash, nonce, raw_tx_hex, submitted_at_ms,
   confirmed_block_number, confirmed_at_ms, failed_reason, failed_at_ms)`.
4. **Optional**: wire `BroadcastPolicy` through
   `HybridV2KmsSignerBridge` for the Remote signer path when the
   deployment moves to a KMS-backed signer. The `BroadcastSigner`
   trait accepts any implementation.
