# Option Simulation Revert — Selector 0x8baa579f

## Reproduction Summary

During V1F runtime verification (2026-05-21), a live simulation against the deployed
OptionMatchingEngine on Base Sepolia (chain 84532) returned:

- `simulation_status`: `simulation_failed`
- `simulation_revert_selector`: `0x8baa579f`

The simulation was submitted via `POST /options/execution-intents/:id/simulate` with
dummy 65-byte signatures (`0xaa` × 65) after a crossing orderbook fill created a
`calldata_ready` intent.

The same revert was reproduced independently with `cast call` (no transaction):

```
cast call \
  --rpc-url $RPC_URL \
  --from 0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 \
  0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b \
  <executeTrade calldata with dummy sigs>

# Output:
Error: server returned an error response: error code 3: execution reverted, data: "0x8baa579f"
```

## Full Revert Data

```
0x8baa579f
```

Four-byte error selector, no parameters.

## Selector Attribution

| Selector   | Error Signature                        | Source                                          | Match |
|------------|----------------------------------------|-------------------------------------------------|-------|
| 0x8baa579f | `InvalidSignature()`                   | `OptionMatchingEngine.sol:56`                   | **YES** |
| 0xf645eedf | `ECDSAInvalidSignature()`              | OZ ECDSA                                        | no    |
| 0xfce698f7 | `ECDSAInvalidSignatureLength(uint256)` | OZ ECDSA                                        | no    |
| 0xd78bce0c | `ECDSAInvalidSignatureS(bytes32)`      | OZ ECDSA                                        | no    |
| 0x54535301 | `SeriesInactive()`                     | `OptionMatchingEngine.sol:64`                   | no    |
| 0xd69b5379 | `InvalidTrade()`                       | `OptionMatchingEngine.sol:58`                   | no    |
| 0x4bd574ec | `BadNonce()`                           | `OptionMatchingEngine.sol:57`                   | no    |
| 0x6f003885 | `MarginRequirementBreached(address)`   | `MarginEngineStorage.sol`                       | no    |

Computed with `cast sig "<error signature>"`. All 347 custom errors in `src/` were enumerated;
`InvalidSignature()` is the unique match for `0x8baa579f`.

## Matched Error

**`InvalidSignature()`** defined at `OptionMatchingEngine.sol:56`:

```solidity
error InvalidSignature();
```

Emitted at `OptionMatchingEngine.sol:415`:

```solidity
if (!_verify(t.buyer, digest, buyerSig)) revert InvalidSignature();
```

## Execution Flow Leading to This Revert

`OptionMatchingEngine.executeTrade()` checks in order:

1. `onlyExecutor` — PASS. `from = 0xc35F...23C27` is the deployer/owner with
   `isExecutor[owner] = true` set in the constructor.
2. `whenNotPaused` — PASS. Contract is not paused.
3. `_requireEngineSet()` — PASS. MarginEngine is wired.
4. `_validate(t)`:
   - `_isStructurallyValid(t)` — PASS. All required fields present and non-zero.
   - `_isDeadlineValid(t)` — PASS. `deadline = 0` means no expiry.
   - `_validateSeriesMetadata(t)`:
     - `getSeriesIfExists(optionId)` → `exists = true` ✓
     - `series.isActive` → `true` ✓ ← **series activation confirmed**
     - metadata match (underlying, settlementAsset, expiry, strike, isCall, contractSize) → PASS ✓
5. `hashTrade(t)` — computes EIP-712 digest over the `OptionTrade` struct.
6. `_verify(t.buyer, digest, buyerSig)`:
   - `ECDSA.tryRecover(digest, 0xaaaa…aa)` → recovers a random address, not `t.buyer`
   - Returns `false` → **`revert InvalidSignature()`** ← simulation stops here

The simulation reaches step 6. `SeriesInactive()` is no longer thrown.

## Confirmed Series Active

The activated optionId:

```
24145907678156652148089862289363692212069910767044828147380657249455352740183
```

Verified active on-chain: `_validateSeriesMetadata` would have reverted with
`SeriesInactive()` (selector `0x54535301`) before reaching `hashTrade` if the series
were not active. The simulation reaching `InvalidSignature()` is direct evidence that
`OptionProductRegistry.getSeriesIfExists(optionId).isActive == true` on Base Sepolia.

## Note on Prior Session Selector

The conversation context from V1F stated that the *previous* (pre-activation) simulation
reverted with "`SeriesInactive()` selector `0x023b0878`." This is inconsistent: 
`cast sig "SeriesInactive()"` = `0x54535301`, not `0x023b0878`. The hex `0x023b0878` 
does not match any error in the local Solidity source. The correct `SeriesInactive()`
selector is `0x54535301`; `0x023b0878` appears to have been a computation error in the
prior session's notes and is not used anywhere in backend code.

## Is This Acceptable with Dummy Signatures?

**Yes.** NEXT_TASK.md explicitly documents this as an acceptable outcome:

> If dummy signatures are used:
> - `simulation_failed`
> - `reason != SeriesInactive()`
> - **likely `InvalidSignature` or equivalent**

`0x8baa579f` = `InvalidSignature()` satisfies all three conditions.

No code patch is required in either the backend or Solidity. The backend correctly
propagates the revert selector from the RPC response into `simulation_revert_selector`.

## Direct eth_call Result

```
Contract : 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b (OptionMatchingEngine, Base Sepolia)
from     : 0xc35F7A8A103A9A4464adfaa76B9B514093D23C27 (executor/owner)
calldata : executeTrade(<real series params>, 0xaaaa…aa, 0xaaaa…aa)
result   : revert 0x8baa579f  (InvalidSignature())
```

Reproduced with `cast call --rpc-url $RPC_URL --from ... <OME_address> <calldata>`.
Exit code 1, no transaction broadcast.

## Next Steps Toward simulation_ok

To obtain `simulation_ok`, the buyer and seller must sign the correct EIP-712 digest
with their actual private keys before the calldata is submitted for simulation.

Required:
1. **Funded accounts**: buyer and seller must each have margin deposited in `CollateralVault`
   sufficient to meet the `RiskModule` margin requirement for the option position.
2. **Valid signatures**: both buyer and seller sign the `OptionTrade` EIP-712 struct
   with domain `{name: "DeOptV2-OptionMatchingEngine", version: "1", chainId: 84532,
   verifyingContract: 0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b}`.
3. **Matching nonces**: `buyerNonce` and `sellerNonce` in the signed struct must equal
   the current on-chain `nonces[address]` values.
4. **Non-expired deadline**: `deadline` must be `0` or a future timestamp.

With valid signatures and sufficient margin, the simulation would proceed past
`_verify()` and `_consumeNonces()` into `marginEngine.applyTrade()`, which could
succeed (`simulation_ok`) or revert with a margin/risk error.

The next likely revert after signatures are valid is `MarginRequirementBreached(address)`
(selector `0x6f003885`) if the test accounts have no deposited collateral.
