# AWS KMS signer runtime config template

**Posture:** TEMPLATE / DOC ONLY. No real env values. No real
credentials. No `.env` edited.

**Anchors:**
- `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision
  (Option A vs Option B).
- `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM role + KMS
  policy.
- `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — feature
  flag + `aws-sdk-kms` integration.

## 0. Hard rules (this doc)

```text
no AWS_ACCESS_KEY_ID                     ✅
no AWS_SECRET_ACCESS_KEY                 ✅
no AWS_SESSION_TOKEN                     ✅
no raw private key                       ✅
no real KMS key id / alias / ARN         ✅
no real AWS account ID                   ✅
no real region beyond placeholders       ✅
no .env edit                             ✅
no real EVM address                      ✅
```

## 1. Config layers

The signer track has THREE distinct config layers. Each layer's
values live in a DIFFERENT location with a DIFFERENT credential
delivery model.

| Layer | Lives in | Holds | Read by |
|---|---|---|---|
| **Backend app config** | `.env` / operator secret store | `BACKEND_SIGNER_*` typed-config values (no AWS specifics) | DeOpt backend process |
| **Signer runtime config** (Option A: same as backend; Option B: separate microservice) | Container env / instance metadata | `AWS_REGION` + KMS key alias + signer-microservice mTLS cert paths if Option B | The signer runtime — backend (Option A) OR microservice (Option B) |
| **AWS IAM role config** | AWS account / IAM service | Role trust policy + permissions | AWS — automatically by `aws-sdk-kms` via instance metadata / IRSA / OIDC |

The backend app NEVER reads AWS credentials. AWS credentials come
from the IAM role attached to the runtime.

## 2. Layer 1 — Backend app config

These env keys are read by the backend's `config::env::Config` loader
at startup. Defaults and validation are pinned in `src/config/env.rs`
+ `src/execution/config.rs`.

```bash
# ── Backend signer mode ─────────────────────────────────────────────
# Required on mainnet: `remote`. Refused: `local_dev` on chain_id 8453.
BACKEND_SIGNER_MODE=remote

# ── Signer microservice / direct-KMS endpoint URL ───────────────────
# Option A (direct AWS KMS): use a sentinel like https://aws-kms-direct
#   to indicate the AWS SDK client is used directly; future
#   refactor may make this URL optional for direct mode.
# Option B (signer microservice): the microservice's mTLS endpoint URL.
# Either way: NEVER a URL with embedded credentials/tokens.
BACKEND_SIGNER_ENDPOINT=https://signer.<operator-domain>.invalid

# ── Signer provider kind ────────────────────────────────────────────
# Selects which `PluggableSignerProvider` to wire. `aws_kms` chooses
# the AWS-KMS-backed adapter. `Mock` REFUSED on mainnet at startup.
BACKEND_REMOTE_SIGNER_PROVIDER=aws_kms

# ── Per-request timeout (ms) ────────────────────────────────────────
# Range 100..=30000; default 2500.
BACKEND_SIGNER_TIMEOUT_MS=2500

# ── Configured executor EVM address ─────────────────────────────────
# Derived offline from the KMS public key via the address-derivation
# flow in AWS_KMS_OPERATOR_SETUP_PACK.md §3. Cross-checked at
# health_check; mismatch → PostSignFromMismatch + fail-closed.
# DO NOT put the real production address in tracked files; operator
# binder + runtime secret store only.
EXECUTOR_FROM_ADDRESS=0x<placeholder-do-not-commit-real-address>

# ── Real-broadcast enable (mainnet) ─────────────────────────────────
# Required true to broadcast. Refused at startup on mainnet if
# any of EXECUTOR_PRIVATE_KEY set / BACKEND_SIGNER_MODE not remote /
# BACKEND_SIGNER_ENDPOINT empty.
EXECUTOR_REAL_BROADCAST_ENABLED=true

# ── Executor chain id ───────────────────────────────────────────────
# 8453 = Base mainnet; 84532 = Base Sepolia; 31337 = anvil.
EXECUTOR_CHAIN_ID=8453

# ── Hard rule (mainnet) ─────────────────────────────────────────────
# Do NOT set EXECUTOR_PRIVATE_KEY on mainnet. Startup refuses.
# Do NOT set EXECUTOR_ALLOW_LOCAL_SIGNER on mainnet (no effect; LocalDev
# refused on mainnet unconditionally).
```

**Backend app config does NOT include:**

* `AWS_ACCESS_KEY_ID` — comes from IAM role.
* `AWS_SECRET_ACCESS_KEY` — comes from IAM role.
* `AWS_SESSION_TOKEN` — comes from IAM role STS short-lived creds.
* `AWS_REGION` — read by the AWS SDK from the signer runtime layer,
  NOT the backend app. (In Option A both layers coincide; the AWS SDK
  reads from the same env, but the backend's typed config loader never
  consumes the value.)
* `AWS_KMS_KEY_ID` / `AWS_KMS_KEY_ALIAS` — signer runtime layer
  config.
* `EXECUTOR_PRIVATE_KEY` — refused on mainnet.

## 3. Layer 2 — Signer runtime config

### 3.1 Option A — Backend direct to AWS KMS

The backend process IS the signer runtime. The AWS SDK client reads
standard AWS SDK env keys from the SAME process env, but the backend's
typed config loader never sees them.

```bash
# ── AWS SDK auto-discovery env keys (NOT read by backend loader) ────
AWS_REGION=<AWS_REGION>

# ── KMS key alias / id ──────────────────────────────────────────────
# Passed to the AwsKmsSignerProvider constructor by operator wiring
# code at the rehearsal Phase 3 cutover. NOT a backend env key.
# Format: arn / key UUID / alias name. Example placeholder:
AWS_KMS_SIGNER_KEY_ID=alias/<KMS_KEY_ID_OR_ALIAS>

# ── No long-lived creds ─────────────────────────────────────────────
# IAM role attached to EC2 instance / EKS pod (IRSA) / ECS task / Fargate.
# AWS SDK reads STS short-lived creds from the EC2 instance metadata
# endpoint OR the AWS_WEB_IDENTITY_TOKEN_FILE env (IRSA) automatically.
```

### 3.2 Option B — Signer microservice

The MICROSERVICE's runtime — NOT the backend's — holds these keys.

```bash
# ── Microservice AWS config ─────────────────────────────────────────
AWS_REGION=<AWS_REGION>
AWS_KMS_SIGNER_KEY_ID=alias/<KMS_KEY_ID_OR_ALIAS>

# ── Microservice mTLS endpoint ──────────────────────────────────────
SIGNER_MICROSERVICE_BIND_ADDR=0.0.0.0:8443
SIGNER_MICROSERVICE_TLS_CERT_PATH=/etc/signer/tls/server.pem
SIGNER_MICROSERVICE_TLS_KEY_PATH=/etc/signer/tls/server-key.pem
SIGNER_MICROSERVICE_CA_BUNDLE_PATH=/etc/signer/tls/ca-bundle.pem

# ── Backend allowlist ───────────────────────────────────────────────
# Comma-separated list of accepted backend client cert subjects.
SIGNER_MICROSERVICE_ALLOWED_CALLERS=CN=deopt-backend-prod

# ── Policy layer ────────────────────────────────────────────────────
SIGNER_MICROSERVICE_ALLOWED_CHAIN_IDS=8453
SIGNER_MICROSERVICE_ALLOWED_TARGETS=0x<placeholder-ome-address>
SIGNER_MICROSERVICE_ALLOWED_SELECTORS=0x031f77b3,0x<placeholder-rfq-selector>
SIGNER_MICROSERVICE_MAX_VALUE_WEI=0
```

### 3.3 Credential delivery preference

Order of preference (best → worst):

1. **IRSA (EKS service account → IAM role)** — short-lived OIDC token,
   automatically rotated by Kubernetes. No env keys in the pod spec.
2. **ECS task role** — short-lived creds via task metadata endpoint.
   No env keys.
3. **EC2 instance role** — short-lived creds via instance metadata
   endpoint. No env keys.
4. **Lambda role** — short-lived creds, but cold-start latency
   unsuitable for signing critical path.
5. **AWS_WEB_IDENTITY_TOKEN_FILE** — for non-EKS OIDC providers
   (acceptable).
6. **AWS_PROFILE + credentials file** — for operator-side break-glass
   workstations ONLY. Never on production runtime.
7. **AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY** — **NEVER on
   production runtime**. Operator-side console session only.

The AWS SDK's `aws-config` crate auto-discovers from this preference
chain. The backend explicitly does NOT call `Credentials::new`; the
SDK figures it out.

## 4. Hard rules for runtime config

| Rule | Enforcement |
|---|---|
| No raw private key in `.env` | Backend startup refuses `EXECUTOR_PRIVATE_KEY` on mainnet. |
| No AWS long-lived creds in app `.env` | Operational discipline — verified by CI lint + `.env.example` audit + secret-scanning hook. |
| No `EXECUTOR_FROM_ADDRESS` in tracked file | Operational discipline — production env file lives in operator secret store. |
| No mock provider on mainnet | Startup refuses `BACKEND_REMOTE_SIGNER_PROVIDER=mock` on chain_id 8453. |
| No `LocalDev` mode on mainnet | Startup refuses `BACKEND_SIGNER_MODE=local_dev` on chain_id 8453. |
| Timeout bound | Startup refuses `BACKEND_SIGNER_TIMEOUT_MS` outside 100..=30000. |
| No SDK retry on `Sign` | Adapter contract per `MAINNET_SIGNER_VENDOR_ADAPTER_REQUIREMENTS.md §2.11` — adapter rejects retry attempts even if SDK config requests one. |

## 5. Direct-backend launch (Option A) — IAM role attachment

| Deployment substrate | Attachment |
|---|---|
| EC2 | Attach `<SIGNER_RUNTIME_ROLE_NAME>` as instance profile. |
| EKS | Annotate Kubernetes ServiceAccount: `eks.amazonaws.com/role-arn: arn:aws:iam::<AWS_ACCOUNT_ID>:role/<SIGNER_RUNTIME_ROLE_NAME>`. Bind pod to ServiceAccount. |
| ECS / Fargate | `taskRoleArn` in task definition = `arn:aws:iam::<AWS_ACCOUNT_ID>:role/<SIGNER_RUNTIME_ROLE_NAME>`. |

Backend container env contains `AWS_REGION` + `AWS_KMS_SIGNER_KEY_ID`.
NO access keys.

## 6. Signer-microservice launch (Option B) — IAM role attachment

The IAM role is attached to the MICROSERVICE's runtime — not the
backend's. The backend's runtime has ZERO KMS permissions.

| Deployment substrate | Attachment |
|---|---|
| EC2 | Microservice instance profile = `<SIGNER_RUNTIME_ROLE_NAME>`. Backend instance profile = a separate role with NO KMS permissions. |
| EKS | Microservice ServiceAccount IRSA → `<SIGNER_RUNTIME_ROLE_NAME>`. Backend ServiceAccount → a separate role with NO KMS permissions. |
| ECS / Fargate | Microservice task role = `<SIGNER_RUNTIME_ROLE_NAME>`. Backend task role = separate. |

mTLS between backend and microservice; backend's client cert subject
listed in `SIGNER_MICROSERVICE_ALLOWED_CALLERS`.

## 7. Sample `.env.example` skeleton

`.env.example` is committed; `.env` is NEVER committed. Skeleton:

```bash
# ── Required env keys (placeholders only — copy to .env locally and
#    fill in operator secret store at deployment time) ──────────────
BACKEND_SIGNER_MODE=remote
BACKEND_REMOTE_SIGNER_PROVIDER=aws_kms
BACKEND_SIGNER_ENDPOINT=https://signer.<operator-domain>.example
BACKEND_SIGNER_TIMEOUT_MS=2500
EXECUTOR_REAL_BROADCAST_ENABLED=false  # flip to true at rehearsal Phase 3 cutover
EXECUTOR_CHAIN_ID=8453
EXECUTOR_FROM_ADDRESS=0x<placeholder>
# Do NOT set: EXECUTOR_PRIVATE_KEY
# Do NOT set: AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_SESSION_TOKEN
```

## 8. Cross-links

* `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
* `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy
  templates.
* `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail + alerts.
* `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — preflight verification.
* `docs/BACKEND_AWS_KMS_PRODUCTION_TRANSPORT_RESULT.md` — feature
  flag.
* `docs/MAINNET_SIGNER_STAGING_REHEARSAL_PLAN.md` — phase ladder.
