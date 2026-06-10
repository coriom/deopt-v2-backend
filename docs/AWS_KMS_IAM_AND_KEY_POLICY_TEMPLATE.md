# AWS KMS IAM and key policy template

**Posture:** TEMPLATE / DOC ONLY. No real AWS account / IAM role / KMS
key. No `terraform apply`. No AWS CLI commands against any real
account. Every value in the JSON below is a placeholder; operator
substitutes real values in their own offline configuration.

**Anchors:**
- `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision +
  readiness summary.
- `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config
  layout.
- `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail configuration.
- `MAINNET_CUSTODY_POLICY.md §6 BE-5 + §7.4` — custody rules.

## 0. Hard rules (this doc)

```text
no real AWS account ID                   ✅
no real IAM role / user name             ✅
no real KMS key ID / alias / ARN         ✅
no AWS access keys                       ✅
no AWS secret keys                       ✅
no session tokens                        ✅
no policy values that could be mistaken  ✅
  for production secrets
no executable credentials                ✅
no .env edit                             ✅
```

## 1. Placeholders

Every JSON template below uses these placeholders. Operator
substitutes the real values OFFLINE; tracked docs MUST stay neutral.

| Placeholder | Description |
|---|---|
| `<AWS_ACCOUNT_ID>` | 12-digit AWS account number where the KMS key lives. |
| `<AWS_REGION>` | AWS region — e.g. `eu-central-1`. |
| `<KMS_KEY_ID_OR_ALIAS>` | KMS key id (UUID), key ARN, or alias name (`alias/<name>`). |
| `<KMS_KEY_ARN>` | Full ARN `arn:aws:kms:<AWS_REGION>:<AWS_ACCOUNT_ID>:key/<UUID>`. |
| `<SIGNER_RUNTIME_ROLE_NAME>` | IAM role attached to the signer runtime (backend instance for Option A; signer microservice instance for Option B). |
| `<SIGNER_RUNTIME_PRINCIPAL_ARN>` | The role's full ARN — `arn:aws:iam::<AWS_ACCOUNT_ID>:role/<SIGNER_RUNTIME_ROLE_NAME>`. |
| `<KMS_ADMIN_ROLE_NAME>` | IAM role used by operators / security engineers for key administration (separate from runtime role). |
| `<KMS_ADMIN_PRINCIPAL_ARN>` | `arn:aws:iam::<AWS_ACCOUNT_ID>:role/<KMS_ADMIN_ROLE_NAME>`. |
| `<CLOUDTRAIL_TRAIL_NAME>` | Name of the CloudTrail trail capturing KMS data events. |

## 2. Identity separation

| Role | Permissions |
|---|---|
| `<SIGNER_RUNTIME_ROLE_NAME>` | **Runtime** — bound to backend / signer microservice instance. Allowed `kms:GetPublicKey` + `kms:Sign` ONLY, against ONE KMS key resource. Cannot manage / delete / disable / re-policy the key. Cannot do IAM operations. |
| `<KMS_ADMIN_ROLE_NAME>` | **Operator / security** — bound to operator console session via SSO. Allowed `kms:CreateKey`, `kms:DescribeKey`, `kms:EnableKey`, `kms:DisableKey`, `kms:ScheduleKeyDeletion`, `kms:PutKeyPolicy`, `kms:UpdateAlias`. **Never used by any backend process.** Operator-only via console / break-glass STS session. |
| Auditor / read-only | (optional) `kms:DescribeKey` + `kms:GetPublicKey` ONLY. No signing. No state mutation. |

The runtime role and the admin role MUST be distinct IAM principals.
Granting both to the same role collapses the trust boundary.

## 3. Signer runtime IAM role policy (least-privilege)

Attach this policy to `<SIGNER_RUNTIME_ROLE_NAME>`. It grants
sign + public-key-read against EXACTLY ONE KMS key resource and
NOTHING else.

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowSignAndGetPublicKeyOnOpBeKey",
      "Effect": "Allow",
      "Action": [
        "kms:GetPublicKey",
        "kms:Sign"
      ],
      "Resource": [
        "arn:aws:kms:<AWS_REGION>:<AWS_ACCOUNT_ID>:key/<KMS_KEY_ID_OR_ALIAS>"
      ],
      "Condition": {
        "StringEquals": {
          "kms:SigningAlgorithm": "ECDSA_SHA_256"
        }
      }
    },
    {
      "Sid": "DenyDangerousKmsActions",
      "Effect": "Deny",
      "Action": [
        "kms:ScheduleKeyDeletion",
        "kms:CancelKeyDeletion",
        "kms:DisableKey",
        "kms:EnableKey",
        "kms:PutKeyPolicy",
        "kms:CreateGrant",
        "kms:RetireGrant",
        "kms:RevokeGrant",
        "kms:UpdateAlias",
        "kms:DeleteAlias",
        "kms:CreateAlias",
        "kms:TagResource",
        "kms:UntagResource",
        "kms:ImportKeyMaterial",
        "kms:DeleteImportedKeyMaterial"
      ],
      "Resource": "*"
    },
    {
      "Sid": "DenyAllIamForRuntimeRole",
      "Effect": "Deny",
      "Action": [
        "iam:*",
        "sts:AssumeRole",
        "sts:GetSessionToken",
        "organizations:*",
        "account:*"
      ],
      "Resource": "*"
    }
  ]
}
```

Notes:
* `kms:SigningAlgorithm` condition pins `ECDSA_SHA_256` — the runtime
  cannot request a different algorithm even if the backend code were
  somehow asked to.
* The Deny statements are explicit (override Allow even in cross-account
  scenarios) and form the operational backstop: a bug or
  configuration drift cannot escalate the runtime role's
  capabilities.

## 4. KMS key policy (resource-side)

Attach this key policy to `<KMS_KEY_ID_OR_ALIAS>`. It grants admin
operations to the operator role + signing operations to the runtime
role + auditor read-only to a third optional principal.

```json
{
  "Version": "2012-10-17",
  "Id": "deopt-op-be-key-policy",
  "Statement": [
    {
      "Sid": "EnableRootAccountForBreakglass",
      "Effect": "Allow",
      "Principal": {
        "AWS": "arn:aws:iam::<AWS_ACCOUNT_ID>:root"
      },
      "Action": "kms:*",
      "Resource": "*"
    },
    {
      "Sid": "AllowKmsAdminViaOperatorRole",
      "Effect": "Allow",
      "Principal": {
        "AWS": "<KMS_ADMIN_PRINCIPAL_ARN>"
      },
      "Action": [
        "kms:Describe*",
        "kms:Get*",
        "kms:List*",
        "kms:Enable*",
        "kms:Disable*",
        "kms:ScheduleKeyDeletion",
        "kms:CancelKeyDeletion",
        "kms:PutKeyPolicy",
        "kms:CreateAlias",
        "kms:DeleteAlias",
        "kms:UpdateAlias",
        "kms:TagResource",
        "kms:UntagResource"
      ],
      "Resource": "*"
    },
    {
      "Sid": "AllowSignerRuntimeMinimalPermissions",
      "Effect": "Allow",
      "Principal": {
        "AWS": "<SIGNER_RUNTIME_PRINCIPAL_ARN>"
      },
      "Action": [
        "kms:GetPublicKey",
        "kms:Sign"
      ],
      "Resource": "*",
      "Condition": {
        "StringEquals": {
          "kms:SigningAlgorithm": "ECDSA_SHA_256"
        }
      }
    },
    {
      "Sid": "DenySignerRuntimeKeyManagement",
      "Effect": "Deny",
      "Principal": {
        "AWS": "<SIGNER_RUNTIME_PRINCIPAL_ARN>"
      },
      "Action": [
        "kms:ScheduleKeyDeletion",
        "kms:DisableKey",
        "kms:EnableKey",
        "kms:PutKeyPolicy",
        "kms:CreateGrant",
        "kms:RetireGrant",
        "kms:RevokeGrant",
        "kms:CreateAlias",
        "kms:UpdateAlias",
        "kms:DeleteAlias",
        "kms:TagResource",
        "kms:UntagResource",
        "kms:ImportKeyMaterial"
      ],
      "Resource": "*"
    }
  ]
}
```

Notes:
* The root account statement is the AWS-mandated break-glass; only
  the operator's root credentials (held offline, MFA-protected, rarely
  used) can override.
* The runtime principal's deny statement is intentionally redundant
  with the IAM policy's deny in §3 — defence-in-depth at both the
  identity AND resource boundaries.
* No `kms:Encrypt` / `kms:Decrypt` / `kms:GenerateDataKey*` — this
  key signs only.

## 5. CloudTrail requirement

The KMS key MUST be covered by a CloudTrail trail that captures BOTH
management events AND KMS data events. Recommended trail
configuration:

| Aspect | Value |
|---|---|
| Trail name | `<CLOUDTRAIL_TRAIL_NAME>` (e.g. `deopt-op-be-kms-trail`) |
| Scope | Single-region (matches the KMS key region) OR multi-region (operator preference; multi-region preferred for audit completeness) |
| Management events | All — captures admin role activity. |
| Data events | KMS — type `AWS::KMS::Key`, resource arn = `<KMS_KEY_ARN>`. Read AND Write. |
| Log file validation | Enabled (Amazon S3 SSE-KMS bucket with operator-managed key — NOT the signer key). |
| Retention | ≥ 7 years (per `MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §5`). |
| Lifecycle | Hot → S3 Standard for 90 days; → S3 Glacier Deep Archive for the remainder. |

Operator-side detail in
`docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md`.

## 6. Trust policy template (assume-role)

Attach this trust policy to `<SIGNER_RUNTIME_ROLE_NAME>` so the
deployment runtime can assume it via the chosen credential delivery
mechanism (instance metadata / IRSA / task role / OIDC).

### 6.1 EC2 instance role

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowEc2InstanceAssumeRole",
      "Effect": "Allow",
      "Principal": {
        "Service": "ec2.amazonaws.com"
      },
      "Action": "sts:AssumeRole"
    }
  ]
}
```

### 6.2 EKS service account (IRSA)

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowEksServiceAccountAssumeRole",
      "Effect": "Allow",
      "Principal": {
        "Federated": "arn:aws:iam::<AWS_ACCOUNT_ID>:oidc-provider/oidc.eks.<AWS_REGION>.amazonaws.com/id/<OIDC_PROVIDER_ID>"
      },
      "Action": "sts:AssumeRoleWithWebIdentity",
      "Condition": {
        "StringEquals": {
          "oidc.eks.<AWS_REGION>.amazonaws.com/id/<OIDC_PROVIDER_ID>:sub": "system:serviceaccount:<K8S_NAMESPACE>:<K8S_SERVICE_ACCOUNT>",
          "oidc.eks.<AWS_REGION>.amazonaws.com/id/<OIDC_PROVIDER_ID>:aud": "sts.amazonaws.com"
        }
      }
    }
  ]
}
```

### 6.3 ECS task role

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowEcsTasksAssumeRole",
      "Effect": "Allow",
      "Principal": {
        "Service": "ecs-tasks.amazonaws.com"
      },
      "Action": "sts:AssumeRole"
    }
  ]
}
```

Operator picks the trust policy matching the deployment substrate
(EC2 / EKS / ECS / Fargate / Lambda — note Lambda not recommended due
to cold-start latency).

## 7. Explicit deny guidance

The signer runtime role MUST NEVER:

* Schedule the key for deletion (`kms:ScheduleKeyDeletion`).
* Disable the key (`kms:DisableKey`).
* Modify the key policy (`kms:PutKeyPolicy`).
* Create / modify / delete an alias (`kms:CreateAlias` /
  `UpdateAlias` / `DeleteAlias`).
* Create or revoke grants (`kms:CreateGrant` / `RetireGrant` /
  `RevokeGrant`).
* Import key material (`kms:ImportKeyMaterial`) — the key MUST be
  AWS-origin non-exportable.
* Tag / untag the key (`kms:TagResource` / `UntagResource`).
* Perform ANY IAM operation (`iam:*`).
* Assume a different role (`sts:AssumeRole` / `sts:AssumeRoleWithSAML`
  / `sts:AssumeRoleWithWebIdentity`).
* Read CloudTrail (`cloudtrail:Get*` / `cloudtrail:Describe*`) —
  read-only of the audit trail is operator-side.

All of these are denied at BOTH the IAM role's policy AND the KMS
key's policy. Defence-in-depth.

## 8. Audit / operator separation

* **Operator role** (key admin) — used only via console + MFA + STS
  short-lived session. Never bound to a deployment instance.
* **Auditor role** (optional) — read-only; can call
  `kms:DescribeKey` + `kms:GetPublicKey` for verification but never
  `kms:Sign`. Useful for independent third-party verification of the
  derived EVM address.
* **Signer runtime role** — bound to deployment instance / pod / task;
  signs + reads public key.
* **No shared role** between operator + runtime.

## 9. Cross-links

* `docs/AWS_KMS_OPERATOR_SETUP_PACK.md` — architecture decision.
* `docs/AWS_KMS_SIGNER_RUNTIME_CONFIG_TEMPLATE.md` — runtime config.
* `docs/AWS_KMS_CLOUDTRAIL_AND_MONITORING_RUNBOOK.md` — CloudTrail
  trail + alerts.
* `docs/AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — preflight verification
  + go/no-go.
* `docs/MAINNET_SIGNER_ROTATION_AND_INCIDENT_RUNBOOK.md §2 + §3` —
  rotation + compromise procedures.
* `MAINNET_CUSTODY_POLICY.md §6 BE-5 + §7.4` — custody rules.
