# DeOpt D2 live fee and frontend acceptance V1

## Verified governance state

On Base Sepolia, the read-only backend D2 verifier authenticated the five
reviewed D2 execution receipts and read all PERP tiers from deployed D2:
maker 0 ppm and taker 200 ppm. The recipient is ProtocolTimelock. A separate
pinned RPC check authenticated all six S2 receipts by their Safe success and
Timelock execution logs, confirmed that the six operations were consumed, and
read D3 mUSDC as explicitly enabled with a 5 bps seizure spread. At the pinned
block, D4's four restrictions and D6's pause remained active; D1 had zero
markets and C02 remained queued. See the adjacent machine-readable manifest
for the block hashes and transaction IDs. These reads do not authorize trading.

Run the backend verifier with `BASE_SEPOLIA_RPC_URL` and
`D2_PERP_TIER_S2_TX_HASHES` (five comma-separated reviewed public hashes).
It requires chain 84532, canonical receipts and block hashes, Timelock
consumption, fresh readbacks, the fee recipient and zero-rebate funding posture.
It has no signer or broadcast code.

## Receipt and fee accounting

For a direct `executeTradeFromIntents`, the existing proof decodes both signed
intents, recovers the wallets and checks the event digests. For a wrapper with
multiple D6 calls, D6 emits `TradeExecutedFromIntents` immediately after D4
`applyTrade` returns. D4 emits its wallet-bearing `TradeExecuted` after its
D2 fee calls; the subsequent risk checks are `view`. The new verifier requires
the adjacent D4/D6 pair, exact market/size/price and canonical receipt log
provenance. It associates the D2 charge in that fill's log segment. A missing,
duplicate or divergent event fails closed. This proves wallet settlement and
the on-chain intent digests. It does **not** recover wrapper call signatures
or prove independent on-chain subaccount balances. The subaccount field stays
null for canonical wallet fees.

All fills in one receipt now commit in one PostgreSQL transaction. The test
forces a later fill failure and verifies rollback of the earlier fee row,
then checks retry idempotency, wallet isolation and keyset pagination. The
read-only `GET /accounts/{wallet}/perps/canonical-fees` route accepts `limit`
(1–100) and an optional `(before_block,before_log_index)` cursor. It rejects
subaccount filters. Rows are receipt-verified at ingestion; the API explicitly
requires a fresh chain recheck for current canonicality.

The frontend's Perps `My Fees` tab displays six-decimal mUSDC receipt amounts
at wallet scope. It rejects malformed or cross-wallet API responses and shows
disconnected, wrong-network and unavailable states. The production frontend
has no configured trading API endpoint, so the view remains unavailable until
a separately reviewed backend deployment. The Perps trade submit control
remains disabled.

## Acceptance limits and next action

The multi-fill positive and negative tests use source-compatible synthetic
receipt logs, not public fills. C02 is unexecuted and market 1 is absent; no
real D6 fill can be used for acceptance. The current Docker image was built
and started non-root with a read-only filesystem, trading and broadcasters
disabled. Its health/readiness endpoints and wallet fee API passed against a
disposable PostgreSQL fixture. A local Chromium test reached two fixture fee
rows through the actual frontend, backend HTTP service and database. These
tests do not prove a real D6 transaction. A local Anvil transaction exercising
multiple real D6 fills through a wrapper remains the next implementation gate.
A canonical wallet fee is never assigned to a local economic subaccount without
separate source-backed proof. Keep public broadcasting and trading disabled
independently of D2 verification.

Before remote hosting, require an isolated Anvil receipt-to-API acceptance,
an explicit wallet/subaccount product decision, authenticated read-only API
access appropriate to public chain data, persistent PostgreSQL with tested
backups, RPC failover, bounded workers, monitoring and an operator-controlled
activation gate. Vercel remains the frontend host; no Rust service was
deployed remotely in this milestone.
