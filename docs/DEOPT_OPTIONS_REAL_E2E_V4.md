# DEOPT_OPTIONS_REAL_E2E_V4

This milestone is local only. No Base Sepolia transaction, production deployment, or GitHub push was performed.

## Executed local chain and database path

Run `bash scripts/run-options-anvil-indexer-v4.sh` from the backend repository. It requires local `anvil`, `forge`, `cast`, Docker, PostgreSQL client, and Rust. The runner binds Anvil and a disposable PostgreSQL 16 to `127.0.0.1`, checks both ports are free, broadcasts the Foundry script only to that Anvil, runs the Rust integration test, then stops the resources it created. It never targets the hosted backend or database. Oracle prices and token balances in `OptionsLocalAnvilE2E.s.sol` are deterministic local test fixtures.

The passing run used Anvil chain 31337. `executeTrade` produced a successful receipt and seven logs; the Rust indexer persisted 11 events across the deployment and trade blocks. It read `MarginEngine.getPositionQuantity` as buyer +1 and seller -1, and the receipt-backed API returned matching buyer/seller contract counts from PostgreSQL. The test then advanced Anvil time, set a local settlement price, submitted real `settleAccount` transactions for both wallets, indexed two `AccountSettled` logs, and read both positions as zero. A snapshot revert and replacement blocks removed the orphaned trade from PostgreSQL. The following hashes are ephemeral local proof, not public transactions:

- Trade: `0x5cb2a1537a5ad1083f95a7a7dc9964e316f31c253ce10aaa3167ef64b4eeb35e`
- Block hash: `0x2f9550170d150ea484cce8709735aa7eb8f1cc5b5ac8cea0e55219f82a476a09`
- Settlement price: `0x8d59c5877aae1d4c0e4d45bf35c8159c04f25cb147b842771dd9e98ab908a839`
- Seller settlement: `0x7e1e0ae60614afec40bb1bd26006085bfe7696ddef623f1a4cb23c0549b86f55`
- Buyer settlement: `0x4b97378a6a206b6529430aa03fee2e55d99a5b00a54438e79ba970a56cd09de6`

The indexer now persists a canonical block hash with its cursor. On a cursor hash mismatch, it deletes affected chain events and resets the cursor in one PostgreSQL transaction before replay. Wallet projection reads use only indexed receipts with block hashes, compare open contract counts to MarginEngine, and fail closed on mismatch or a changed cursor hash. The test snapshot predates contract deployment, so the post-reorg API correctly refuses the read with HTTP 503 when MarginEngine disappears. Economic fields remain unverified and absent. The existing wallet session guard still protects the route.

The final one-command run passed one ignored-by-default Rust integration test (`1 passed, 0 failed`) and left neither its PostgreSQL container nor its localhost listeners running. The Foundry broadcast artifact reported `receipt.status=0x1` with seven trade logs. The frontend type check and targeted ESLint check passed. The browser wallet fixture signed EIP-712 cryptographically, but that test did not submit an order to the Rust API.

## Acceptance boundary

The local EIP-712 test and browser wallet signature fixture passed independently. A complete Next.js browser flow that submits the order through Rust, receives the actual receipt, and renders the reconciled position has **not** been demonstrated by these proofs. In particular, the broadcast in this runner is initiated by the Foundry script. Do not label `FULL_E2E`, `PLAYWRIGHT_E2E`, or Base Sepolia trading readiness PASS from this milestone.

The next implementation gate is a test-only, loopback-bound Rust execution adapter that accepts signed orders from the real local Next.js interface, sends them to the local Anvil matching engine, then lets the indexer and authenticated position endpoint update the browser. The adapter must be impossible to enable in production. The economics and replay checks need to run in that same integrated fixture before the user flow can be accepted.
