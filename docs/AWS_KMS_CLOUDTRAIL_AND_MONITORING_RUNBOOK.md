# AWS KMS CloudTrail and monitoring runbook

**Posture:** RUNBOOK / DOC ONLY. No real CloudTrail trail created. No
real CloudWatch alarms / SIEM rules created. No webhook secret. No
`.env` edited. Operator executes the runbook against a real account in
a separately-authorised milestone.

**Anchors:**
- `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
- `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + KMS policy
  templates.
- `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — real
  RequestId extraction.
- `docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md` — backend metric +
  alert taxonomy.
- `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §5` — audit
  log retention requirements.
- `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health endpoint
  surface.

## 0. Hard rules (this doc)

```text
no real CloudTrail trail creation        ✅
no real CloudWatch alarms                ✅
no real SIEM forwarding                  ✅
no webhook secret creation               ✅
no real AWS account ID                   ✅
no real KMS key ID / ARN                 ✅
no AWS access keys                       ✅
no .env edit                             ✅
no live AWS CLI commands                 ✅
```

## 1. CloudTrail requirements

### 1.1 Management events

* **Capture:** ALL `kms:` management events (`CreateKey`,
  `DescribeKey`, `EnableKey`, `DisableKey`, `PutKeyPolicy`,
  `ScheduleKeyDeletion`, `CancelKeyDeletion`, `CreateAlias`,
  `UpdateAlias`, `DeleteAlias`, `TagResource`, etc.).
* **Why:** Operator-side key administration MUST be auditable. Any
  unexpected `DisableKey` / `ScheduleKeyDeletion` / `PutKeyPolicy` is
  an immediate red alert.

### 1.2 Data events

* **Capture:** `kms:Sign` + `kms:GetPublicKey` against the
  specific KMS key resource ARN (`<KMS_KEY_ARN>`).
* **Why:** Every Sign call attributable to the signer runtime. Every
  health-check call attributable to a deployment / operator.
* **Scope:** Read AND Write events on the specific resource. Other
  KMS keys in the account are out of scope.

### 1.3 Log file validation

* **Enable** log file integrity validation (default to ON).
* CloudTrail digest files are signed; integrity verifier rejects
  tampering.

### 1.4 Storage

* S3 bucket with SSE-KMS using a DIFFERENT KMS key (operator-managed,
  NOT the signer key — avoids circular dependency).
* Bucket has Object Lock enabled with retention period 7 years +
  Governance mode (allow operator audit team to verify; prevents
  accidental delete).
* Bucket lifecycle: Standard → Glacier Deep Archive at 90 days.
* Cross-region replication recommended for disaster recovery (a
  second region holds the read-only copy).

### 1.5 Retention

7 years minimum per
`MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §5`. Recommended:
indefinite retention for sealed compromised-window archives.

### 1.6 Forwarding

* CloudTrail → S3 bucket (canonical store).
* S3 → CloudWatch Logs (live forwarding via Lambda subscription
  filter OR EventBridge → CloudWatch Logs target).
* CloudWatch Logs → operator's SIEM (Splunk / Datadog / etc.) via
  cross-account log subscription OR Lambda forwarder. Operator
  preference.

## 2. RequestId correlation

The backend transport `AwsKmsSdkTransport` extracts the real AWS
`RequestId` via `aws_sdk_kms::operation::RequestId` trait per
`BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md`. The id flows
through the audit chain:

```
output.request_id()  (AWS SDK)
  → sanitize_request_id(...)  (5-step pipeline)
  → GetPublicKeyResponse.request_id / SignDigestResponse.request_id
  → PluggableSignResult.provider_request_id + audit_log_id
  → SignerResponse.kms_request_id + audit_log_id + remote_signer_request_id
  → src/options/service.rs::sign_option_execution_via_signer
    INFO log under target "broadcast_signer"
```

Operator correlates a CloudTrail event by:

1. Search backend log for `target=broadcast_signer` + the
   `intent_id` of interest.
2. Read the structured `kms_request_id` field — strip the
   `aws-kms-{sign,get-public-key}-` prefix.
3. Search CloudTrail event lookup by `eventID = <stripped-uuid>`.

If the backend log shows the synthetic `-synthetic-` infix, the
SDK didn't surface a RequestId (transport-shim bug, future SDK
regression, etc.); fall back to timestamp + key id + caller arn
correlation in CloudTrail.

## 3. Alert conditions

Each alert maps to a CloudWatch metric filter on the CloudTrail log
group OR a SIEM rule on the forwarded events.

### 3.1 Critical (page on-call within 5 minutes)

| Alert | Trigger | Reason |
|---|---|---|
| `KMS_UNEXPECTED_DISABLE_KEY` | Any `DisableKey` event on the signer key from any principal OTHER than the operator role. | Indicates compromise OR misconfiguration. |
| `KMS_UNEXPECTED_SCHEDULE_KEY_DELETION` | Any `ScheduleKeyDeletion` event. | Compromise OR human error; pending-window deletion still needs CancelKeyDeletion. |
| `KMS_PUT_KEY_POLICY` | Any `PutKeyPolicy` event. | Key policy modification by anyone (including admin) requires review. |
| `KMS_UNEXPECTED_REGION` | Any `Sign` / `GetPublicKey` event from a region OTHER than the configured `<AWS_REGION>`. | Cross-region call from unexpected client. |
| `KMS_ACCESS_DENIED_BURST` | `>3` `AccessDenied` events on `Sign` within 5 minutes. | IAM drift OR active attack attempt. |
| `KMS_SIGN_FROM_UNKNOWN_ROLE` | Any `Sign` event from a principal OTHER than `<SIGNER_RUNTIME_PRINCIPAL_ARN>`. | Compromise. |
| `KMS_SIGNING_RATE_ANOMALY` | `Sign` rate `>10x` the 7-day baseline. | Backend bug OR replay attack. |

### 3.2 Warning (Discord / ops channel)

| Alert | Trigger | Reason |
|---|---|---|
| `KMS_THROTTLING` | Any `ThrottlingException` on `Sign`. | Vendor side or operator quota; investigate. |
| `KMS_SIGN_LATENCY_P99` | `Sign` p99 latency `>500ms` over 5 minutes. | Vendor degradation or network issue. |
| `KMS_GET_PUBLIC_KEY_FAILURE` | Any `GetPublicKey` failure during a non-startup window. | Probably a health-check failure; the runtime should retry. |
| `KMS_SIGN_OUTSIDE_DEPLOYMENT_WINDOW` | `Sign` event between operator-defined freeze window (e.g. weekends if applicable). | Soft warning. |
| `KMS_SUDDEN_VOLUME_DROP` | `Sign` rate drops by `>90%` vs 7-day baseline for `>15` minutes. | Backend unhealthy. |

### 3.3 Suppression

* Operator-scheduled key administration windows (rotation cycles,
  policy review) should pre-create CloudWatch suppressions to avoid
  paging on-call.
* No alert routes to PagerDuty / Discord automatically — operator
  configures routing per
  `BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 + §4` with their own
  webhook secrets in the operator secret store.

## 4. Mapping to backend observability

The backend already surfaces signer state via Prometheus + the JSON
health endpoint. Cloud-side alerts complement; they do NOT duplicate.

| Cloud signal | Backend signal |
|---|---|
| CloudTrail `Sign` event | `signer_attempted_total{signer_kind="remote"}` increments |
| CloudTrail `Sign` success response | `signer_success_total{signer_kind="remote"}` increments + `last_broadcast_submitted_ms` updates |
| CloudTrail `Sign` AccessDenied | `signer_denied_total{code="caller-unauthorized", signer_kind="remote"}` + `last_signer_error_code="caller-unauthorized"` |
| CloudTrail `Sign` Throttling | `signer_denied_total{code="rate-limit", …}` + `last_signer_error_code="rate-limit"` |
| CloudTrail `Sign` Disabled/Invalid | `signer_denied_total{code="kms-unavailable", …}` + `last_signer_error_code="kms-unavailable"` |
| CloudTrail `Sign` Timeout | `signer_denied_total{code="kms-timeout", …}` + `last_signer_error_code="kms-timeout"` |
| CloudTrail `GetPublicKey` failure | Backend `health_check` returns Err → `/executor/health/v2.signer.signer_address` becomes `null` + `last_signer_error_code` matches the AWS error category |
| CloudTrail key policy modification | NOT surfaced by backend — cloud-side alert ONLY |

### 4.1 `/executor/health/v2.signer.*` mapping

| Field | Cloud equivalent |
|---|---|
| `signer.signer_mode` | always `"remote"` on mainnet |
| `signer.remote_signer_configured` | `BACKEND_SIGNER_ENDPOINT` set |
| `signer.signer_address` | EVM address derived from KMS key via `GetPublicKey` |
| `signer.last_signer_kind` | always `"remote"` |
| `signer.last_signer_success_at_ms` | timestamp of most recent successful CloudTrail `Sign` |
| `signer.last_signer_error_code` | maps to the CloudTrail error category |
| `signer.local_signer_on_mainnet_refused_total` | MUST remain 0; non-zero indicates a defence-in-depth fire (compromise) |

## 5. Audit query patterns

Common forensics queries operators run against CloudTrail (operator
executes these from their console; backend never reads CloudTrail).

### 5.1 "Show me every Sign event for intent_id `<UUID>`"

1. Backend log search: `intent_id=<UUID>` → extract `kms_request_id`.
2. CloudTrail `LookupEvents`:
   ```text
   EventName = Sign
   EventID   = <stripped-uuid-from-kms_request_id>
   ```

### 5.2 "Has anyone other than the signer runtime called Sign?"

CloudTrail Insights / Athena query:

```sql
SELECT eventTime, userIdentity.arn, eventName, errorCode
FROM cloudtrail_logs
WHERE eventTime >= ?
  AND eventName = 'Sign'
  AND resources[1].arn = '<KMS_KEY_ARN>'
  AND userIdentity.arn != '<SIGNER_RUNTIME_PRINCIPAL_ARN>'
ORDER BY eventTime DESC;
```

Should return ZERO rows during normal operation.

### 5.3 "Show me all key administration events for the last 30 days"

```sql
SELECT eventTime, userIdentity.arn, eventName
FROM cloudtrail_logs
WHERE eventTime >= now() - interval '30 days'
  AND eventName IN ('PutKeyPolicy', 'EnableKey', 'DisableKey',
                    'ScheduleKeyDeletion', 'CancelKeyDeletion',
                    'CreateAlias', 'UpdateAlias', 'DeleteAlias',
                    'TagResource', 'UntagResource')
  AND resources[1].arn = '<KMS_KEY_ARN>'
ORDER BY eventTime DESC;
```

## 6. Incident response correlation

When the incident runbook (`MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §3`)
fires, the cloud-side artefacts operator captures:

1. CloudTrail event lookup for the incident window (output to a
   sealed S3 archive).
2. CloudWatch metric snapshot of the incident window.
3. CloudWatch Logs export of the backend logs forwarded to CloudWatch.
4. Backend `tracing` log archive (operator-side).
5. Backend `/metrics` Prometheus snapshot.
6. `/executor/health/v2` snapshot.

All artefacts retained per §5 of the rotation/incident runbook (7
years minimum; indefinite for sealed-incident archives).

## 7. Cross-links

* `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
* `docs/AWS_KMS_IAM_AND_KEY_POLICY_TEMPLATE.md` — IAM + key policy.
* `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
* `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — preflight verification.
* `docs/BACKEND_AWS_KMS_CLOUDTRAIL_REQUEST_ID_RESULT.md` — RequestId
  extraction chain.
* `docs/BACKEND_EXECUTOR_MONITORING_ALERTS_V1.md §3 + §4` — backend
  alert taxonomy.
* `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §3 + §5` —
  incident response + retention.
* `docs/EXECUTOR_HEALTH_ENDPOINT_V2_RESULT.md` — health endpoint.
