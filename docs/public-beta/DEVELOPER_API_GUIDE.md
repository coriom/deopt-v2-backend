# DeOpt V2 — Developer API Guide (Public Beta)

> **Testnet only. No real funds. Unaudited. Experimental. APIs subject to change.**

This guide covers the public, read-mostly surface of the DeOpt V2 backend, suitable for integrators who want to query series, preview quotes, build intents, collect EIP-712 signatures, and observe transaction status — all against the Base Sepolia testnet.

Admin / operator endpoints (`/admin/*`) are **explicitly NOT covered here**. They require an admin bearer token, are not part of the public surface, and may be removed without notice.

---

## 1. Base URL and conventions

```
{{ API_BASE_URL }}        # placeholder — operator to fill before announcing the beta
```

* Base Sepolia (chain id `84532`) only.
* All endpoints return JSON.
* All responses follow a **status envelope**:

  ```json
  {
    "status": "ok" | "partial" | "stale",
    "data":   <endpoint payload>,
    "warnings": [{"code": "...", "message": "..."}],
    "meta": {
      "source":    "internal" | "rpc" | "indexer" | "store",
      "chain_id":  84532,
      "request_id": "<uuid>",
      "generated_at_ms": 1700000000000
    }
  }
  ```

  | `status` | Meaning |
  |---|---|
  | `ok` | All upstream sources returned cleanly. Payload is canonical. |
  | `partial` | Payload is best-effort; at least one upstream returned a soft error. See `warnings`. |
  | `stale` | The payload is from a cache that exceeds the configured freshness window. |

* Error codes you'll see frequently in `warnings`:

  | Code | Meaning |
  |---|---|
  | `INVALID_ADDRESS` | A supplied address failed validation (checksum, length). |
  | `SOURCE_UNAVAILABLE` | The upstream data source (RPC, indexer, store) didn't answer in time. |
  | `RPC_UNAVAILABLE` | The chain RPC didn't respond. |
  | `ORACLE_UNAVAILABLE` | The oracle returned `ok=false` or a stale `updatedAt`. |
  | `ACCOUNT_STATE_UNAVAILABLE` | The MarginEngineLens read-path returned partial state. |
  | `SETTLEMENT_PREVIEW_UNAVAILABLE` | The lens preview returned partial state for a specific position. |
  | `CONFIG_MISSING` | A required env-side address (e.g., lens) was unset; the endpoint returned a soft warning. |

---

## 2. Public endpoints

### 2.1 Health

```
GET /trading/health
```

Returns the overall service health, indexer lag (if available), RPC reachability flag, chain id, and current indexed block. Use this as a first call before integrating; if `status != "ok"`, expect downstream endpoints to be degraded.

Example response (real):

```json
{
  "status": "ok",
  "data": {
    "overall_status": "ok",
    "indexer_lag_blocks": null,
    "rpc_reachable": true,
    "chain_id": 84532,
    "indexed_block": null
  },
  "warnings": [],
  "meta": { "source": "internal", "chain_id": 84532, "request_id": "...", "generated_at_ms": ... }
}
```

### 2.2 Products

```
GET /options/products
```

Returns the list of option products. Each product groups one or more series by underlying / settlement / contract size.

### 2.3 Series detail

```
GET /options/series/:series_id/details
```

Returns the series metadata (underlying, settlement, expiry, strike, isCall, contractSize, isActive) plus oracle freshness flags. If the oracle is stale, `status` will be `partial` and `warnings` will include `ORACLE_UNAVAILABLE` or a `stale oracle` reason.

### 2.4 Account balances

```
GET /accounts/:address/balances
```

Returns the connected wallet's mUSDC EOA balance, allowance to the CollateralVault, and vault deposit. If the lens address is unset on the backend, the endpoint returns `partial` with `CONFIG_MISSING`.

### 2.5 Account portfolio

```
GET /accounts/:address/portfolio
```

Returns the per-series position quantities for the connected wallet, plus account-level risk (equity, margin requirements). If the lens address is unset, returns `partial` with `ACCOUNT_STATE_UNAVAILABLE`.

---

## 3. Trading endpoints (write-ish)

These endpoints write to the backend's DB but do not directly broadcast chain transactions. The actual `executeTrade` broadcast is done by the executor service after both signatures are collected.

### 3.1 Quote preview

```
POST /options/quote-preview
Content-Type: application/json

{
  "series_id":  "0x8b34d095…",
  "side":       "buy" | "sell",
  "quantity":   "1"
}
```

Returns a quote envelope:

```json
{
  "status": "ok",
  "data": {
    "premium_per_contract_native": "1000000",
    "total_premium_native": "1000000",
    "oracle_mark_1e8": "300000000000",
    "deadline_hint":  1781276500
  },
  "warnings": []
}
```

If the oracle is stale, `status` will be `partial` with `ORACLE_UNAVAILABLE`. Don't proceed to intent creation until the operator refreshes the mock oracle.

### 3.2 Create execution intent

```
POST /options/execution-intents
Content-Type: application/json

{
  "series_id":  "0x8b34d095…",
  "buyer":      "0x394291A0…",
  "seller":     "0xb1f1ae6C…",
  "quantity":   "1",
  "premium_per_contract_native": "1000000",
  "buyer_is_maker": false,
  "deadline":   1781276500
}
```

Returns the created intent row, including:

* `intent_id` (UUID — backend's internal id)
* `onchain_intent_id` (32-byte hash — what the EIP-712 typed data references)
* `buyer_nonce`, `seller_nonce` (read from the matching engine at intent creation time)
* `status` (initially `signatures_required`)

### 3.3 Fetch signing payload

```
GET /options/execution-intents/:intent_id/signing-payload
```

Returns the EIP-712 typed data the buyer and seller need to sign. You can pass this directly to your wallet's typed-data signing function (e.g., `wallet_client.signTypedData(...)` in viem).

### 3.4 Submit signature

```
POST /options/execution-intents/:intent_id/signatures
Content-Type: application/json

{
  "role":      "buyer" | "seller",
  "signature": "0x…132 hex chars…"
}
```

The backend verifies the signature off-chain (against the typed data) and stores it. Once both buyer and seller signatures are present, the intent advances to `calldata_ready`, and the executor will broadcast `executeTrade` on its next tick.

### 3.5 Transaction status / lifecycle

```
GET /options/execution-intents/:intent_id/lifecycle
```

Returns the full lifecycle envelope:

```json
{
  "intent_id": "...",
  "status":    "broadcast_confirmed",
  "broadcast": {
    "tx_hash":             "0x748c9484…",
    "confirmation_status": "mined_success",
    "confirmed_block_number": 42750521,
    "gas_used":            683044
  },
  "reconciliation": {
    "status": "reconciled",
    "decoded_event_count": 19,
    "trading_fee_event_count": 2,
    "internal_transfer_event_count": 3
  },
  "events": { "total": 19, "counts_by_event_name": { "OptionTradeExecuted": 1, … } }
}
```

This is the endpoint your UI should poll while the trade is in flight.

---

## 4. EIP-712 domain

```
name              : "DeOptV2-OptionMatchingEngine"
version           : "1"
chainId           : 84532
verifyingContract : 0x5a5EBF9A9CCd7c012518569DE8283982982670f6
```

Domain separator (computed on chain):

```
0x68d1704576b276dd7c71b56f36643fdc3fc7c4cee67830362e3488f207cfbfb8
```

If your wallet computes a different domain separator at signing time, double-check the verifying-contract address. Do NOT sign against the legacy matching engine (`0xf2D1D85…`); signatures for that ME will recover the wrong address inside the canonical matching engine and the trade will revert.

---

## 5. Worked example — full integration

```js
// Pseudocode using viem-style API
const product = await fetch(`${API}/options/products`).then(r => r.json());
const seriesId = product.data[0].series_id;

const quote = await fetch(`${API}/options/quote-preview`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ series_id: seriesId, side: "buy", quantity: "1" })
}).then(r => r.json());

if (quote.status !== "ok") {
  throw new Error("quote not OK: " + JSON.stringify(quote.warnings));
}

const intent = await fetch(`${API}/options/execution-intents`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({
    series_id: seriesId,
    buyer:  myAddress,
    seller: counterparty,
    quantity: "1",
    premium_per_contract_native: quote.data.premium_per_contract_native,
    buyer_is_maker: false,
    deadline: quote.data.deadline_hint
  })
}).then(r => r.json());

const payload = await fetch(`${API}/options/execution-intents/${intent.data.intent_id}/signing-payload`)
  .then(r => r.json());

const signature = await walletClient.signTypedData({
  domain:      payload.data.domain,
  types:       payload.data.types,
  primaryType: payload.data.primaryType,
  message:     payload.data.message
});

await fetch(`${API}/options/execution-intents/${intent.data.intent_id}/signatures`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ role: "buyer", signature })
});

// counterparty does the same → intent moves to calldata_ready
// executor broadcasts → poll lifecycle
let lifecycle;
do {
  await new Promise(r => setTimeout(r, 2000));
  lifecycle = await fetch(`${API}/options/execution-intents/${intent.data.intent_id}/lifecycle`)
    .then(r => r.json());
} while (lifecycle.data.status !== "broadcast_confirmed" && lifecycle.data.status !== "broadcast_failed");
```

---

## 6. Separation between public and admin endpoints

| Path prefix | Audience | Auth |
|---|---|---|
| `/trading/*` | public | none |
| `/options/*` | public (read + write-ish for intents/sigs) | none for read; CAPTCHA / rate limit may apply for writes |
| `/accounts/*` | public (read) | none |
| `/admin/*` | OPERATOR ONLY | admin bearer token (`x-admin-token` header) |
| `/admin/options/events/tick` and `/admin/options/reconciliations/tick` | OPERATOR ONLY | admin bearer; not a public endpoint |

**Never include an admin bearer token in a client-side build, in a screenshot, or in a bug report.** The team will never ask for it.

---

## 7. Rate limits and stability

* The public testnet beta does not have advertised rate limits. The team may add them without notice.
* The backend DB may be reset between iterations. If your integration relies on persistent state, expect occasional intent history loss.
* The backend's chain id is hard-coded to `84532` (Base Sepolia). It will refuse to start with any other chain id.

---

## 8. OpenAPI reference

The full OpenAPI spec lives at:

```
deopt-v2-backend/docs/openapi/trading-api.openapi.json
```

Treat it as the canonical reference. This guide is a narrative companion, not an exhaustive enumeration.

---

## 9. Disclaimer

* **Testnet only.** No mainnet. No real funds.
* **Unaudited.** No external audit has been completed.
* **APIs may change.** Without notice. The beta is explicitly experimental.
* **Not financial advice.** Anything here is illustrative; the protocol is not a regulated venue.

---

**End of developer API guide.**
