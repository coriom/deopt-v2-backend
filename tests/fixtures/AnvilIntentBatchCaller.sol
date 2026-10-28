// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

/// @notice Disposable local Anvil fixture. This contract only forwards an
/// array of already encoded calls in order and reverts atomically on failure.
/// It has no owner, signer, broadcaster or production deployment path.
contract AnvilIntentBatchCaller {
    error BatchCallFailed(uint256 index, bytes revertData);

    function execute(address target, bytes[] calldata calls) external {
        for (uint256 i; i < calls.length; ++i) {
            (bool ok, bytes memory result) = target.call(calls[i]);
            if (!ok) revert BatchCallFailed(i, result);
        }
    }
}
