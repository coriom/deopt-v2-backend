# Mainnet manifest — missing values table

**Posture:** DOC ONLY. READ-ONLY status tracker. **No chain mutation,
no `.env` edit, no Safe-tx, no broadcast, no mainnet, no guessed
mainnet addresses, no secret printed.** Names every missing mainnet
value with status, owner, and the safe way to obtain it.

**Companion:** `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md`.

## 0. Hard rules (this doc)

```text
no guessed mainnet contract address                 ✅
no guessed production signer address                ✅
no real AWS account ID / KMS key id / ARN           ✅
no AWS access key / secret key / session token      ✅
no private custody roster                           ✅
no real production .env values                      ✅
no mainnet contract deployment                      ✅
```

## 1. Status legend

| Status | Meaning |
|---|---|
| **KNOWN** | Value is finalised and publicly anchored (chain-verified or in a tracked doc). |
| **MISSING** | Value will be filled by a downstream milestone; the milestone is identified. |
| **OPERATOR_INPUT_REQUIRED** | Value is operator commercial / legal / custody input; cannot be derived from code or chain. |
| **NOT_APPLICABLE** | Slot is intentionally absent at launch. |
| **BLOCKED_BY_PREVIOUS_STEP** | Value depends on a still-open prerequisite milestone. |

## 2. Secret / commit flags

| Flag | Meaning |
|---|---|
| **PUBLIC** | Value is non-secret; may be committed to tracked docs once known. |
| **NEVER_COMMIT** | Value is secret; lives in operator secret store; tracked docs use placeholders. |
| **OFFLINE_BINDER** | Value is non-secret but per custody policy lives in the operator offline binder; tracked docs use placeholders. |

## 3. Per-field table

Columns: `field` / `repo or doc` / `status` / `owner` / `safe source` /
`secret flag` / `notes`.

### 3.1 Chain / network

| Field | Repo / doc | Status | Owner | Safe source | Secret | Notes |
|---|---|---|---|---|---|---|
| `chain_id = 8453` | code constants | KNOWN | Backend | `MAINNET_CHAIN_ID` const | PUBLIC | Anchored. |
| Mainnet RPC URL | operator secret store | OPERATOR_INPUT_REQUIRED | Operator + Backend ops | Operator selects RPC provider (Alchemy / Infura / Base node). | NEVER_COMMIT (may include token in URL) | Backend reads `RPC_URL` from secret store; never tracked. |
| Block explorer | doc | KNOWN | Operator | `https://basescan.org` | PUBLIC | — |
| `block_range_start` for event indexer | manifest | BLOCKED_BY_PREVIOUS_STEP | Sol + Backend | Set to the block of each contract's mainnet deployment after `MAINNET-DEPLOYMENT` | PUBLIC | Per-contract slot in event indexer config. |

### 3.2 Custody

| Field | Repo / doc | Status | Owner | Safe source | Secret | Notes |
|---|---|---|---|---|---|---|
| `OPS_SAFE_MAINNET` = `0xce0e46Db1072B820CB5eCf30188ED76cb560C932` | docs anchor | KNOWN | Custody / Security | Chain-verified per Cluster 1 result | PUBLIC | Threshold 2/3. |
| `GOV_SAFE_MAINNET` = `0x7C6Ce20eED2b633b4FF4A2e2387E437abc96b166` | docs anchor | KNOWN | Custody / Security | Chain-verified per Cluster 1 result | PUBLIC | Threshold 3/5. |
| OPS Safe owner roster | private | OFFLINE_BINDER | Custody / Security | Operator binder per `MAINNET_CUSTODY_CLUSTER_1_RESOLUTION.private.md` | OFFLINE_BINDER | NEVER committed; chain-verified disjointness recorded. |
| GOV Safe owner roster | private | OFFLINE_BINDER | Custody / Security | Same | OFFLINE_BINDER | NEVER committed. |
| Mainnet DEPLOYER attestation (Q-CD-8) | private | OPERATOR_INPUT_REQUIRED | Custody / Security | Operator captures + records | OFFLINE_BINDER | Still OPEN per Cluster 1 result. |
| Treasury Safe address | manifest | MISSING | Operator + Custody | Created under `MAINNET-TREASURY-SAFE-CREATION-PACKET` | PUBLIC after creation | Pending. |
| Treasury Safe owner roster | private | OPERATOR_INPUT_REQUIRED | Custody / Security | Operator binder | OFFLINE_BINDER | Pending. |
| Insurance Fund operator policy | manifest | MISSING | Operator + Security | Drafted under `MAINNET-INSURANCE-OPERATOR-POLICY-PACKET` | PUBLIC for policy summary; private for signer roster | Pending. |
| Mainnet Timelock instance | deopt-v2-sol/deployments/mainnet.template.json | MISSING | Sol + Operator | Deployed under `MAINNET-V2G-Y-OWNERSHIP-MIGRATION` | PUBLIC after deployment | Per `MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md`. |
| Timelock `MIN_DELAY` value | manifest | OPERATOR_INPUT_REQUIRED | Operator + Security | Operator decides per security profile | PUBLIC | Per `Q-CD-G6` decision. |
| Timelock proposers / executors | manifest | BLOCKED_BY_PREVIOUS_STEP | Operator | OPS Safe + GOV Safe (already known) | PUBLIC | Filled by OPS/GOV addresses. |
| Production OPTION backend executor EVM address | manifest + env | OPERATOR_INPUT_REQUIRED | Operator + Security | Derived from `kms:GetPublicKey` AFTER operator provisions mainnet KMS key per `AWS_KMS_OPERATOR_SETUP_PACK.md` | OFFLINE_BINDER + env (PUBLIC after derivation; placeholder until then) | NEVER guessed; cross-checked at backend startup. |
| Production PERP backend executor EVM address | manifest + env | NOT_APPLICABLE | n/a | Deferred per Q-CD-6 | n/a | PERP backend deferred. |

### 3.3 Contracts

Mainnet equivalents of every contract listed in
`mainnet.template.json`. For each: status MISSING + owner Sol +
filled by mainnet deployment + flag PUBLIC.

| Field | Status | Notes |
|---|---|---|
| `OptionMatchingEngine` (OME) mainnet address | MISSING | Per `MAINNET-DEPLOYMENT`. Address recorded in manifest after deployment. |
| `ProtocolFeeVault` (PFV) mainnet address | MISSING | Same. |
| `FeesManagerV2` mainnet address | MISSING | Same. |
| `CollateralVault` (CV) mainnet address | MISSING | Same. |
| `RiskGuardian` (RG) mainnet address | MISSING | Same. |
| `OptionProductRegistry` mainnet address | MISSING | Same. |
| `OptionMatchingEngineFactory` mainnet address (if applicable) | MISSING | Same. |
| Tokens (USDC etc.) | KNOWN | Pre-existing on Base mainnet; addresses already in `mainnet.template.json`. |
| Oracles (Chainlink price feeds) | KNOWN | Pre-existing on Base mainnet. |
| Old PerpEngine address (`OLD_PERP_ENGINE_ADDRESS`) | NOT_APPLICABLE | No mainnet OLD PERP deployment exists. |

### 3.4 Backend

| Field | Repo / doc | Status | Owner | Safe source | Secret | Notes |
|---|---|---|---|---|---|---|
| `BACKEND_SIGNER_MODE=remote` | `.env` (operator-side) | KNOWN | Backend | Set by operator at deploy | PUBLIC | Required on mainnet. |
| `BACKEND_REMOTE_SIGNER_PROVIDER=aws_kms` | `.env` (operator-side) | KNOWN | Backend | Set by operator at deploy | PUBLIC | Per Q-CD-5 technical recommendation. |
| `BACKEND_SIGNER_ENDPOINT` mainnet URL | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator | Operator chooses Option A direct vs Option B microservice URL | NEVER_COMMIT (URL may carry routing token) | Per `AWS_KMS_OPERATOR_SETUP_PACK.md §2`. |
| `BACKEND_SIGNER_TIMEOUT_MS=2500` | `.env` (operator-side) | KNOWN | Backend | Default 2500 ms; range 100..=30000 | PUBLIC | — |
| `EXECUTOR_FROM_ADDRESS` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator | Derived from KMS GetPublicKey after operator setup | NEVER_COMMIT (placeholder in tracked .env.example) | — |
| `EXECUTOR_PRIVATE_KEY` | n/a | NOT_APPLICABLE | n/a | MUST be unset on mainnet | NEVER_COMMIT | Backend refuses startup if set on chain_id 8453. |
| `EXECUTOR_MAX_GAS_LIMIT` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator + Security | Operator selects safety ceiling | PUBLIC | — |
| `EXECUTOR_MAX_FEE_PER_GAS_WEI` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator + Backend ops | Operator's gas budget policy | PUBLIC | — |
| `EXECUTOR_MAX_PRIORITY_FEE_PER_GAS_WEI` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator + Backend ops | Same | PUBLIC | — |
| `RPC_URL` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator | Operator's RPC provider | NEVER_COMMIT | — |
| `DATABASE_URL` mainnet | `.env` (operator-side) | OPERATOR_INPUT_REQUIRED | Operator | Operator's DB | NEVER_COMMIT | — |
| `PROTOCOL_FEE_VAULT_ADDRESS` mainnet | `.env` (operator-side) | BLOCKED_BY_PREVIOUS_STEP | Backend | After PFV mainnet deployment | PUBLIC | Typed config consumed at startup. |
| `OPTION_EVENT_INDEXER_*` addresses mainnet | `.env` (operator-side) | BLOCKED_BY_PREVIOUS_STEP | Backend | After contract mainnet deployment | PUBLIC | — |

### 3.5 Frontend / admin

| Field | Status | Notes |
|---|---|---|
| Admin token | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT; operator secret store. |
| Admin proxy SSR cert paths | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT. |
| Frontend mainnet RPC URL | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT (may carry token). |
| Frontend block explorer URL | KNOWN | `https://basescan.org`. PUBLIC. |

### 3.6 Monitoring

| Field | Status | Notes |
|---|---|---|
| PagerDuty service key | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT. |
| Discord webhook URL | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT. |
| Prometheus scrape config | OPERATOR_INPUT_REQUIRED | Operator-side deployment config; not in this repo. |
| CloudTrail trail name | OPERATOR_INPUT_REQUIRED | Operator-side. |
| CloudWatch / SIEM forwarding config | OPERATOR_INPUT_REQUIRED | Operator-side. |

### 3.7 AWS KMS

| Field | Status | Notes |
|---|---|---|
| AWS account ID | OPERATOR_INPUT_REQUIRED | NEVER_COMMIT. Per `AWS_KMS_OPERATOR_SETUP_PACK.md`. |
| KMS key id / alias | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER (non-secret per AWS docs, but operator policy keeps it in offline binder). |
| KMS key ARN | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER. |
| `<SIGNER_RUNTIME_ROLE_NAME>` | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER. |
| `<SIGNER_RUNTIME_PRINCIPAL_ARN>` | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER. |
| `<KMS_ADMIN_ROLE_NAME>` | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER. |
| `<CLOUDTRAIL_TRAIL_NAME>` | OPERATOR_INPUT_REQUIRED | OFFLINE_BINDER. |
| AWS access keys / secret keys / session tokens | NOT_APPLICABLE | NEVER created for production runtime; STS short-lived creds via IAM role. |

## 4. Per-row breakdown by status

| Status | Count |
|---|---|
| KNOWN | 8 |
| MISSING | 9 |
| OPERATOR_INPUT_REQUIRED | 18 |
| NOT_APPLICABLE | 4 |
| BLOCKED_BY_PREVIOUS_STEP | 4 |
| OFFLINE_BINDER (additional flag) | applies to private rosters + offline-policy AWS slots |

## 5. Hard rules summary

* No mainnet contract address is guessed; all are MISSING until the
  `MAINNET-DEPLOYMENT` milestone fills them.
* No production signer EVM address is guessed; it is
  OPERATOR_INPUT_REQUIRED and derived offline from `kms:GetPublicKey`.
* No real AWS account ID / KMS key id / ARN appears in tracked docs;
  all are OPERATOR_INPUT_REQUIRED + OFFLINE_BINDER.
* No real RPC URL / DATABASE_URL / admin token / private key / webhook
  secret appears; all are NEVER_COMMIT.
* No private custody roster is committed; OFFLINE_BINDER.

## 6. Owner summary

| Owner | Domain |
|---|---|
| **Custody / Security** | All custody-related rosters, OPS / GOV / Treasury / Insurance, DEPLOYER attestation. |
| **Operator** | AWS account / IAM / KMS provisioning, RPC URL, DATABASE_URL, gas budget policy, deployment env. |
| **Sol** | All contract deployments + manifest fill. |
| **Backend** | Typed config defaults, validation, startup guards. |
| **Frontend** | Admin RBAC config, SSR proxy, mainnet UI deployment. |
| **Audit (external)** | External audit engagement closure (P0-1). |

## 7. Cross-links

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — verification commands
  per category.
* `MAINNET_GO_NO_GO_CRITERIA.md` — criteria gating mainnet activation.
* `deopt-v2-sol/docs/MAINNET_MANIFEST_TODO_INVENTORY.md` — manifest
  template TODOs (76 slots).
* `deopt-v2-sol/docs/MAINNET_MANIFEST_DEPENDENCY_SNAPSHOT_AFTER_CUSTODY_CLUSTERS.md`
  — per-slot blocker state.
* `deopt-v2-backend/docs/AWS_KMS_OPERATOR_SETUP_PACK.md §1 + §2` —
  operator-input map.
