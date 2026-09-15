# PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_DURABILITY_PG_V1

Durability + Postgres hardening on top of
`PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_WORKER_V1`. Closes the
persist-before-send crash window, adds an explicit `Prepared` state,
distinguishes ambiguous vs deterministic RPC failures, and moves
uniqueness + multi-worker nonce safety from Rust in-memory checks to
Postgres constraints.

## New status: `Prepared`

```
Pending → DryRun → CalldataReady → SimulationOk
                                       ↓
                                    Prepared    ← raw envelope + tx_hash
                                       ↓          persisted in DB
                                   Submitted    ← RPC returned OK or
                                   ↓       ↓      idempotent-replay class
                              Confirmed  Failed
```

A `Prepared` row means: **the exact raw signed EIP-1559 envelope is
durable in Postgres, along with its `keccak(raw_tx)` tx_hash and the
allocated executor nonce.** Whether the RPC ever accepted the tx is
unknown from this state alone; the reconciler resolves.

## Byte-identical rebroadcast semantics

**Invariant**: for a `Prepared` row that has not reached a receipt,
the reconciler MAY rebroadcast `raw_tx_hex` verbatim. Because the
envelope is byte-identical:

- `keccak(raw_tx)` is stable → tx_hash unchanged
- `nonce` unchanged → no risk of two nonces from one intent
- Signature unchanged → no risk of a different envelope hash

Rebroadcast is bounded by `BroadcastPolicy::rebroadcast_max_attempts`
(default 8). Beyond that, the row stays `Prepared` and requires
operator intervention.

The reconciler NEVER:
- allocates a new nonce
- builds a different envelope
- generates a different tx_hash
- creates a second economic fill

## Ambiguous RPC failure taxonomy

`SendErrorClass` classifies `eth_sendRawTransaction` errors into four
buckets:

| Class | Meaning | State transition |
|---|---|---|
| `AlreadyKnown` | tx already in node mempool/on-chain | fall through to receipt poll |
| `NonceTooLow` | account nonce advanced past ours (earlier attempt succeeded) | fall through to receipt poll |
| `ReplacementUnderpriced` | same-nonce replacement collision | Prepared → reconciler resolves |
| `DeterministicReject` | insufficient funds / invalid signature / malformed tx / gas too low | Failed (durable) |
| `Ambiguous` | timeout / connection reset / 502 / 504 / EOF | Prepared → reconciler resolves |

**Only** `DeterministicReject` transitions to `Failed` on the send
path. Everything else stays `Prepared` because the node may have
accepted the tx after all.

## Postgres schema

Migration `0063_execution_intent_broadcasts.sql` creates:

```sql
CREATE TABLE execution_intent_broadcasts (
    intent_id                   TEXT PRIMARY KEY REFERENCES execution_intents(intent_id) ON DELETE CASCADE,
    chain_id                    BIGINT NOT NULL,
    executor_address            TEXT NOT NULL,
    target_address              TEXT NOT NULL,
    tx_hash                     TEXT NOT NULL,
    nonce                       BIGINT NOT NULL,
    raw_tx_hex                  TEXT NOT NULL,
    status                      TEXT NOT NULL,      -- prepared | submitted | confirmed | failed
    prepared_at_ms              BIGINT NOT NULL,
    first_submission_at_ms      BIGINT,
    last_send_at_ms             BIGINT,
    send_attempts               INTEGER NOT NULL DEFAULT 0,
    receipt_block_number        BIGINT,
    receipt_status              BIGINT,
    confirmed_at_ms             BIGINT,
    failure_class               TEXT,
    failure_reason              TEXT,
    failed_at_ms                BIGINT,
    updated_at_ms               BIGINT NOT NULL
);

CREATE UNIQUE INDEX uq_execution_intent_broadcasts_tx_hash
    ON execution_intent_broadcasts (tx_hash);

CREATE UNIQUE INDEX uq_execution_intent_broadcasts_executor_nonce
    ON execution_intent_broadcasts (chain_id, executor_address, nonce);

CREATE INDEX idx_execution_intent_broadcasts_status
    ON execution_intent_broadcasts (status);
CREATE INDEX idx_execution_intent_broadcasts_prepared_at_ms
    ON execution_intent_broadcasts (prepared_at_ms);
```

### Uniqueness guarantees

- `intent_id` PRIMARY KEY: one durable broadcast per canonical
  execution identity.
- `uq_execution_intent_broadcasts_tx_hash`: no two intents can share
  a tx envelope.
- `uq_execution_intent_broadcasts_executor_nonce`: two backend workers
  racing to allocate the same executor nonce collide on this
  constraint — the loser sees `BroadcastRejected` and can safely
  retry with a fresh nonce.

### Advisory lock for nonce allocation

`record_prepared_transaction` acquires
`pg_advisory_xact_lock(executor_advisory_lock_key(chain_id, executor))`
inside the same transaction, so concurrent workers serialize their
inserts and the unique-index collision becomes rare. The unique
index is defense-in-depth: even if the advisory lock were removed,
two workers CANNOT emit distinct transactions with the same nonce.

## Multi-worker nonce safety

Two layers:

1. **Serialization**: `pg_advisory_xact_lock` on
   `keccak256("deopt.execution.broadcast.executor_lock.v1" ‖ chain_id
   ‖ executor_addr)[..8]` orders concurrent inserts.
2. **DB-level constraint**: `uq_execution_intent_broadcasts_executor_nonce`
   catches any race that slips past (a). The offending worker sees
   `BackendError::BroadcastRejected("execution_intent_broadcasts unique
   constraint violation…")` and treats it as an idempotent duplicate.

Never depends solely on Rust in-memory checks.

## Signer identity triad

`validate_signer_triad(executor_from_address, hv2_signer_expected_address,
hv2_executor_address)` fails startup if any pair disagrees. The
production wiring MUST call this before the broadcast worker is
started. Case-insensitive; strips `0x` prefix.

The runtime signer bind check in `preflight_static` provides
defense-in-depth: `signer.address()` must equal
`config.executor_from_address` at each broadcast attempt.

## Crash matrix — durable convergence

| Boundary | DB state | Chain state | Restart behavior | Byte-identical resend OK? | New nonce OK? |
|---|---|---|---|---|---|
| A. before tx construction | (none) | none | pick up next tick | n/a | n/a |
| B. after construction, before persistence | (none) | none | pick up next tick | n/a | n/a (nothing persisted) |
| C. after Prepared row, before RPC send | `Prepared`, `send_attempts=0` | none | reconcile: no receipt → rebroadcast byte-identical raw_tx | **YES** | NO |
| D. send with deterministic reject | `Failed`, `failure_reason` set | none | terminal; requires reset | NO | NO |
| E. send with ambiguous transport failure | `Prepared`, `send_attempts≥1` | maybe accepted | reconcile: poll receipt first; if None, rebroadcast byte-identical | **YES** | NO |
| F. node accepted but response lost | `Prepared`, `send_attempts≥1` | tx in mempool/mined | reconcile finds receipt → Confirmed | NO (receipt found first) | NO |
| G. after RPC OK, before status update | `Prepared` | tx in mempool | reconcile: receipt found → Confirmed | NO | NO |
| H. after Submitted, before receipt | `Submitted` | tx in mempool | reconcile: poll receipt until mined | NO | NO |
| I. after receipt available, before DB Confirmed | `Submitted` | mined status=1 or 0 | reconcile: finalize atomically | NO | NO |
| J. after Confirmed | `Confirmed` | mined status=1 | idempotent no-op | NO | NO |

**No boundary requires manual DB deletion.** Every crash boundary
converges via the reconciler.

## Receipt + PME event verification (staged wiring)

The `expected_pme_topic0` field on `BroadcastPolicy` holds the
canonical `TradeExecuted` event topic0
(`0x5018a0a7…dfedb3f80`). Once `EthLogsProvider` is threaded through
the receipt confirmation path, `finalize_receipt` will additionally
verify:

- receipt.tx_hash matches derived envelope
- receipt.status == 1
- log emitter address == PME
- at least one log with topic0 == `PME_TRADE_EXECUTED_TOPIC0` OR
  `PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0`
- (optional) intent_id topic[1] matches canonical execution identity

Absence of the expected event with status=1 is a
`ReconciliationError` (does not silently mark Confirmed).

The topic constants are pinned as public consts so future migrations
can validate them against the Solidity ABI:

```rust
pub const PME_TRADE_EXECUTED_TOPIC0: [u8; 32] = [0x50, 0x18, 0xa0, 0xa7, ...];
pub const PME_TRADE_EXECUTED_FROM_INTENTS_TOPIC0: [u8; 32] = [0x56, 0x0e, 0xbd, 0x5f, ...];
```

## On-chain intentFilled preflight

`preflight_intent_filled(rpc, pme, intent_hash)` returns the current
`intentFilled[hash]` value from PME. If non-zero AND matches the
signed intent size, the backend can skip broadcasting (the tx would
definitionally revert). The on-chain guard in PME's
`executeTradeFromIntents` remains authoritative — this is
operational optimization only.

## Access-gate posture (unchanged)

- **Public Perps remains OFF.** Fail-closed at `src/api/routes.rs:3076-3078`.
- **Funding remains OFF.**
- **Collateral activation unchanged.**
- **Base mainnet untouched.** ChainId preflight refuses non-84532.
- Frontend fail-closed gate intact
  (`NEXT_PUBLIC_PERPS_TICKET_ENABLED` defaults false).

## Env vars unchanged from V1

See
[PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_WORKER_V1](./PERPS_BASE_SEPOLIA_BACKEND_BROADCAST_WORKER_V1.md#environment-variables-names-only)
for the canonical env var reference. No new env vars in this
milestone.

## Follow-on work

1. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_TRADER_FIXTURES_V1`** — deliberately
   fund two closed-test trader wallets with mUSDC collateral. Required
   before any real Base Sepolia trade smoke test.
2. **`PERPS_BASE_SEPOLIA_CLOSED_TEST_SMOKE_ORDER_V1`** — pre-sign a
   matched-pair `PerpOrderIntent` bundle against market 1 (ETH-PERP),
   route via the durable broadcast worker, verify receipt.
3. **PME event verification wiring** — thread `EthLogsProvider`
   through `BroadcastPolicy::finalize_receipt` so the event-topic
   check runs on every receipt. Requires extending `ConfirmationReceipt`
   with logs OR a separate `eth_getLogs` call keyed on `block_hash`.
4. **Reconciler tick loop** — wire `reconcile_unfinalized` into a
   background tokio task at startup, with jitter + backoff.
