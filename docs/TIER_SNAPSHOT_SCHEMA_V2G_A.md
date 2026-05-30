# V2G-A — Tier Snapshot JSON Schema

Defines the JSON artifact produced by
`backend::fees::tier_snapshot::generate_tier_snapshot`. The artifact
is one half of the V2G-A handoff to the Merkle pipeline; the other
half is the Merkle root + per-row proofs documented in
`docs/TIER_MERKLE_REBATE_SYSTEM_V2G_A.md`.

This doc is the source of truth for the row schema; the Rust
struct `TierSnapshotRow` carries `#[derive(Serialize, Deserialize)]`
and uses snake_case field names so the JSON wire format and the
Rust shape are identical.

## Top-level shape

The artifact is a JSON array of rows, sorted by `trader` (ascending,
lowercase hex). No envelope object; the sort key is part of the
deterministic-root contract (rows feed into the Merkle tree in the
same order as the snapshot, so any reordering produces a different
root).

```json
[
  { "trader": "0x000…01", "...": "..." },
  { "trader": "0x000…42", "...": "..." }
]
```

## Row fields

| Field | Type (JSON) | Source | Notes |
| --- | --- | --- | --- |
| `trader` | string (0x-prefixed lowercase hex, 42 chars) | input | Sort key. |
| `option_28d_volume_1e8` | string (decimal `u128`) | input | OPTION venue volume contribution, `1e8`-scaled. Stringified so JS clients don't truncate. |
| `perp_28d_volume_1e8` | string (decimal `u128`) | input | Same for PERP. |
| `total_28d_volume_1e8` | string (decimal `u128`) | derived | `option + perp`, saturating-add. |
| `volume_share_ppm` | u32 | input | 28-day venue share in ppm (`1 % = 10 000`). |
| `staked_deopt_1e8` | string (decimal `u128`) | input | DEOPT stake, `1e8`-scaled token units. |
| `option_tier` | u8 (0..=4) | derived | `resolve_tier_with_eligibility(OptionOrderbook, ...)`. |
| `perp_tier` | u8 (0..=4) | derived | `resolve_tier_with_eligibility(PerpOrderbook, ...)`. |
| `option_maker_ppm` | i32 (signed) | canonical | From the launch schedule; negative for rebate tiers (−50, −25, −10, 0, 50 across T4..T0). |
| `option_taker_ppm` | u32 | canonical | 75, 100, 125, 150, 250 across T4..T0. |
| `perp_maker_ppm` | i32 (signed) | canonical | −100, −75, −50, 0, 50 across T4..T0. |
| `perp_taker_ppm` | u32 | canonical | 150, 175, 200, 250, 300 across T4..T0. |
| `option_rfq_maker_discount_bps` | u32 | canonical | 10 000, 7 500, 5 000, 2 500, 0 across T4..T0. |
| `option_rfq_taker_discount_bps` | u32 | canonical | 7 500, 5 000, 2 500, 1 000, 0 across T4..T0. |
| `valid_from` | u64 (UNIX seconds) | config | Operator-provided; mirrors `setMerkleRoot.validFrom`. |
| `valid_until` | u64 (UNIX seconds) | config | Mirrors `setMerkleRoot.validUntil`. |

## Field source legend

- **input** — sourced from `TraderInputs`; the caller of
  `generate_tier_snapshot` is responsible for sourcing these from
  the persisted fee ledger / on-chain DEOPT balances / share
  computation. The backend does **not** invent values; if a real
  data source is unavailable, callers pass deterministic test
  fixtures and document them.
- **derived** — computed by the snapshot generator from the
  inputs (`total_28d_volume_1e8`, both tier numbers).
- **canonical** — looked up by tier number against the launch
  schedule (`src/fees/schedule.rs::launch_tier`). These fields are
  **observability-only**; on-chain fees are determined by
  `FeesManagerV2.getFeeProfile(tier, product)`, not by the
  snapshot. They are denormalised into the row so operator
  dashboards can render the table without a separate join.

## Worked example

Inputs:

```text
TraderInputs {
    account: 0xab,
    option_volume_28d_1e8: 12_500_000 * 1e8,
    perp_volume_28d_1e8:   12_500_000 * 1e8,
    volume_share_ppm: 0,
    staked_deopt_1e8: 0,
}
SnapshotConfig {
    valid_from:  1_700_000_000,
    valid_until: 1_700_000_000 + 7 * 86_400,
}
```

OR-eligibility computation:
- combined volume = 25 000 000 × 1e8 ≥ Tier 4 threshold ✓
- share = 0, stake = 0 (no contribution)
- highest qualifying tier = **4** (both products)

Resulting JSON row:

```json
{
  "trader": "0x00000000000000000000000000000000000000ab",
  "option_28d_volume_1e8": "1250000000000000",
  "perp_28d_volume_1e8":   "1250000000000000",
  "total_28d_volume_1e8":  "2500000000000000",
  "volume_share_ppm": 0,
  "staked_deopt_1e8": "0",
  "option_tier": 4,
  "perp_tier":   4,
  "option_maker_ppm": -50,
  "option_taker_ppm": 75,
  "perp_maker_ppm": -100,
  "perp_taker_ppm": 150,
  "option_rfq_maker_discount_bps": 10000,
  "option_rfq_taker_discount_bps": 7500,
  "valid_from":  1700000000,
  "valid_until": 1700604800
}
```

## Determinism contract

1. **Ordering.** Rows are sorted by `trader` ascending. Any caller
   that perturbs the input ordering after generation breaks the
   Merkle root downstream. The generator never re-sorts after
   computing fee profiles, so the JSON order is the Merkle leaf
   order.
2. **Field encoding.** `u128` fields are stringified;
   `u32`/`u64`/`i32` use JSON numbers (their value ranges fit in
   `Number` exactly). The `trader` address is always lowercase hex
   with `0x` prefix and 40 hex chars; the address byte order is
   big-endian (the standard Ethereum hex form).
3. **No floats.** Anywhere. The schema deliberately uses ppm /
   bps integer units so JSON serialisation is lossless.

## Versioning

This is the V2G-A version of the schema. Any future field
additions land as **new optional fields appended to the row**;
removing or renaming an existing field is a breaking change and
must come with a new milestone (V2G-B+) and a coordinated update
to consumers (admin UI, Merkle pipeline, operator dashboards).

The Rust struct is annotated with `#[derive(Serialize,
Deserialize)]` without `#[serde(deny_unknown_fields)]`, so
forward-compatibility is permissive at parse time.
