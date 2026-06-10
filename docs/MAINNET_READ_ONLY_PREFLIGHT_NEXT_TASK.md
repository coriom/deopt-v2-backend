# MAINNET-READ-ONLY-PREFLIGHT — next-task prompt

**Posture:** DOC ONLY (this file is the copy-paste prompt for milestone
M9 — `MAINNET-READ-ONLY-PREFLIGHT`). No source code modified by this
document. No real chain or AWS interaction performed by writing this
file.

**Anchors:**
- `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
- `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — the verification
  checklist to execute.
- `MAINNET_GO_NO_GO_CRITERIA.md` — pass criteria.
- `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — verify against this
  table.
- `MAINNET_NEXT_SAFE_MILESTONES.md` — this is milestone M9 in the DAG.

## 0. How to use

Copy the prompt block in §1 into the next operator-milestone command.
The prompt defaults to STRICTLY read-only checks. It explicitly
forbids any chain transaction, Safe-tx, AWS resource creation, or
`.env` edit. The operator runs the checklist against real mainnet RPC
+ real backend deployment (preflight mode; `EXECUTOR_REAL_BROADCAST_ENABLED=false`)
+ optionally against real AWS account (`kms:GetPublicKey` and
`iam:simulate-principal-policy` ONLY; never `kms:Sign`).

## 1. Copy-paste prompt

```text
Workspace root is ~/DEOPT.

Execute MAINNET-READ-ONLY-PREFLIGHT only.

This is a READ-ONLY mainnet preflight milestone.

Do not send any chain transaction.
Do not send any Sepolia transaction.
Do not send any mainnet transaction.
Do not create or update any Safe transaction.
Do not create any AWS resource.
Do not modify any AWS resource.
Do not call kms:Sign at any point.
Do not edit `.env` in this repo (operator may inspect production env
in operator secret store but never edit / commit it here).
Do not print secrets.
Do not print real AWS account IDs.
Do not print real KMS key IDs / ARNs.
Do not print the production EVM signer address into tracked logs.
Do not deploy any contract.
Do not run forge script broadcasts.
Do not modify source code.

Current state:

* MAINNET-AUDIT-MANIFEST-PREFLIGHT-PACK is closed.
* AWS-KMS-OPERATOR-SETUP-EXECUTION is closed (operator-side; real
  AWS account + IAM + KMS key + CloudTrail provisioned per
  AWS_KMS_OPERATOR_SETUP_PACK.md).
* MAINNET-SIGNER-REHEARSAL-PHASE-2-EXECUTION Variant A (no-broadcast)
  is closed.
* MAINNET-AUDIT-EXT-KICKOFF is closed; auditor findings published.
* MAINNET-V2G-Y-OWNERSHIP-MIGRATION is closed (Timelock deployed +
  ownership transferred per MAINNET_V2G_Y_OWNERSHIP_MIGRATION_PLAN.md).
* MAINNET-TREASURY-SAFE-CREATION-PACKET is closed (if applicable).
* MAINNET-INSURANCE-OPERATOR-POLICY-PACKET is closed (if applicable).
* MAINNET-DEPLOYMENT-MANIFEST-FILL is closed.
* Mainnet contracts (OME / PFV / FM_V2 / CV / RG / OptionProductRegistry)
  are deployed on Base mainnet in a SEPARATELY-runbook'd operation
  that precedes this milestone.
* OPS Safe (0xce0e46Db…0C932), GOV Safe (0x7C6Ce20e…b166) are known +
  chain-verified.
* Backend deployment is configured for mainnet WITH
  EXECUTOR_REAL_BROADCAST_ENABLED=false (preflight mode).
* No mainnet broadcast is authorised by this milestone.
* No Sepolia broadcast is authorised by this milestone.

Goal:
Execute every check in MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md
against real mainnet RPC + real backend deployment + real AWS account
(GetPublicKey ONLY). Verify against MAINNET_MANIFEST_MISSING_VALUES_TABLE.md
that every value previously marked MISSING / OPERATOR_INPUT_REQUIRED /
BLOCKED_BY_PREVIOUS_STEP is now KNOWN. Verify against
MAINNET_GO_NO_GO_CRITERIA.md §2 that every GREEN criterion is met.
Surface any YELLOW / RED.

Required Phase A — preflight context capture:

1. Read MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md.
2. Read MAINNET_MANIFEST_MISSING_VALUES_TABLE.md — capture which
   values are now KNOWN that were previously not.
3. Read MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md.
4. Read MAINNET_GO_NO_GO_CRITERIA.md.
5. Confirm operator authorisation for the read-only preflight window
   is captured offline.

Required Phase B — custody checks:

6. Execute §1 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (C1-C14 +
   T1-T5) against mainnet RPC.
7. Capture results into a public-safe result doc (placeholders only
   for any value that would identify private signers; chain-verifiable
   addresses are fine).
8. Confirm OPS Safe threshold 2/3, GOV Safe threshold 3/5, owner
   overlap 0, DEPLOYER not an owner of either Safe, DEPLOYER holds
   no Timelock role.

Required Phase C — contract checks:

9. Execute §2 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (O1-O5 +
   P1-P6 + F1-F4 + V1-V2 + R1-R3 + LI1-LI4) against mainnet RPC.
10. Confirm owners are Timelock for OME/PFV/FM_V2/CV/RG.
11. Confirm paused() is false for OME/PFV/FM_V2/RG.
12. Confirm OME.isExecutor(BE) is the expected boolean (true after
    setExecutor Safe-tx executed; check operator binder for that
    state).
13. Confirm Cluster 4 launch invariant: PFV.rebateReserve = 0 AND
    FM_V2.rebateBudget = 0 AND CV.balances(PFV, asset) = 0 for every
    configured asset.
14. Confirm R5 drift = 0 for every configured asset.

Required Phase D — backend health / observability:

15. Execute §3 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (H1-H19)
    against the backend deployment running in preflight mode.
16. Confirm /health returns ok=true.
17. Confirm /ready returns ready=true.
18. Confirm /executor/health/v2.overall_status == "green".
19. Confirm /executor/health/v2.not_tracked_yet == [].
20. Confirm signer.signer_mode == "remote",
    signer.remote_signer_configured == true,
    signer.signer_address matches the KMS-derived production address
    (cross-check via operator binder; DO NOT log the production
    address into tracked files).
21. Confirm signer.local_signer_on_mainnet_refused_total == 0.
22. Confirm live_provider_config.* == true for PFV / FM_V2 / CV.
23. Confirm chain_state_last_seen.be_balance_floor_wei is bounded
    and matches the configured gas budget.
24. Capture a /metrics scrape; confirm no signer_denied_total
    increments, no policy_data_failures_total increments, no
    fm_v2_*_failures_total increments.

Required Phase E — signer health (no-sign):

25. Execute §4 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (S1-S3).
26. Confirm backend health_check returns Ok with the expected EVM
    address.
27. Verify via CloudTrail event lookup that the health_check window
    triggered ONLY GetPublicKey events and ZERO Sign events.
28. Confirm the recovered address matches EXECUTOR_FROM_ADDRESS.
29. NO kms:Sign call is performed by this milestone. CloudTrail must
    show ZERO Sign events attributed to the signer runtime role for
    the preflight window.

Required Phase F — AWS KMS read-only checks (if operator AWS ready):

30. Execute §5 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (K1-K7).
31. Confirm KMS key state Enabled + correct spec / usage / origin.
32. Confirm derived address matches EXECUTOR_FROM_ADDRESS.
33. Confirm CloudTrail records the K1 + K3 events.
34. Confirm iam:simulate-principal-policy result for the runtime
    role: kms:Sign with ECDSA_SHA_256 allowed; kms:DisableKey /
    ScheduleKeyDeletion / PutKeyPolicy denied; all iam:* denied.

Required Phase G — frontend / admin (if accessible from operator
window):

35. Execute §6 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (A1-A5).
36. Confirm admin dashboard reachable; admin token required (returns
    403 without).
37. Confirm /admin/status returns expected booleans.
38. Confirm /admin/options/executions/<unknown-uuid>/lifecycle returns
    404.
39. Confirm /executor/transactions list returns [] before any
    broadcast.

Required Phase H — cross-cutting must-be-zero checks:

40. Execute §7 of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md (Z1-Z11).
41. Confirm every cumulative counter is 0 at preflight.
42. If any non-zero counter, investigate per
    MAINNET_GO_NO_GO_CRITERIA.md §4 R-series triggers.

Required Phase I — GO criteria evaluation:

43. Cross-check results against MAINNET_GO_NO_GO_CRITERIA.md §2.
44. For each GREEN criterion: mark PASS or FAIL.
45. For each YELLOW criterion: capture context.
46. For each RED criterion: NO-GO + escalate.

Required Phase J — public-safe result doc:

47. Write deopt-v2-backend/docs/MAINNET_READ_ONLY_PREFLIGHT_RESULT.md
    capturing:
    * which sections of MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md
      were executed.
    * GREEN / YELLOW / RED summary against
      MAINNET_GO_NO_GO_CRITERIA.md.
    * counters observed (must be zero per Z1-Z11).
    * Cluster 4 launch invariant result.
    * R5 drift result.
    * IAM simulation result summary.
    * CloudTrail correlation verified.
    * NO Sign event during the preflight window.
    * NO chain transaction sent.
    * NO Safe tx sent.
    * NO `.env` modified.
    * recommended next milestone.
48. The result doc MUST NOT contain real AWS account IDs / KMS key
    IDs / ARNs / production signer EVM address / private custody
    roster. Use placeholders for anything operator-binder.

Required Phase K — RUN_STATE closure:

49. Append a public-safe closure paragraph to RUN_STATE.md noting
    that MAINNET-READ-ONLY-PREFLIGHT executed with the GREEN/YELLOW/RED
    summary. No secrets.

Validation:

50. git diff --check.
51. git status (should show only the new result doc + RUN_STATE
    closure paragraph; no source code modified).
52. Confirm no chain transaction sent.
53. Confirm no Sepolia transaction sent.
54. Confirm no Safe transaction sent.
55. Confirm no AWS resource created or modified.
56. Confirm no kms:Sign call.
57. Confirm no `.env` edited in this repo.
58. Confirm no secrets printed.
59. Confirm no production address committed to tracked logs.
60. Confirm no real AWS account ID / KMS key id / ARN committed.

Forbidden:

* no mainnet tx.
* no Sepolia tx.
* no Safe tx.
* no governance / Timelock / ownership / guardian mutation.
* no rebate reserve allocation.
* no PFV withdrawal.
* no fund movement.
* no `.env` edit.
* no AWS resource creation / modification / deletion.
* no kms:Sign call.
* no real AWS account ID in tracked docs.
* no real KMS key id / ARN in tracked docs.
* no production EVM address in tracked logs.
* no private custody roster disclosure.
* no AWS access keys / secret keys / session tokens in any output.
* no fallback path allowing mainnet local-key signing.
* no source code modification.
* no test suite re-run unless source code accidentally touched.

Hard stops:

* stop if any chain transaction would be sent.
* stop if `.env` would be modified in this repo.
* stop if any AWS resource would be created or modified.
* stop if kms:Sign would be called.
* stop if any secret would be printed.
* stop if real AWS account ID / KMS key id / ARN / production EVM
  address would land in tracked docs.
* stop if any GO criterion fails — escalate per
  MAINNET_GO_NO_GO_CRITERIA.md §4 / §5 / §6 / §7 / §8 / §9 / §10.
* stop if source code modification appears necessary — surface the
  required change as a separate follow-on milestone, do NOT mutate
  source in THIS milestone.

Return final report grouped by:
workspace,
docs inspected,
custody check results (§1 of checklist),
contract check results (§2),
backend health results (§3),
signer health results (§4),
AWS KMS read-only results (§5),
frontend / admin results (§6),
zero-counter results (§7),
GO criteria summary (GREEN / YELLOW / RED count),
RED criteria triggered (if any),
public-safe result doc path,
RUN_STATE update (one-line closure),
files changed (should be 2: result doc + RUN_STATE; nothing else),
validations,
blockers,
next milestone recommendation.
```

## 2. Cross-links

* `MAINNET_AUDIT_MANIFEST_PREFLIGHT_PACK.md` — pack overview.
* `MAINNET_READ_ONLY_PREFLIGHT_CHECKLIST.md` — checklist this prompt
  executes.
* `MAINNET_GO_NO_GO_CRITERIA.md` — criteria.
* `MAINNET_MANIFEST_MISSING_VALUES_TABLE.md` — missing values to
  verify resolved.
* `MAINNET_NEXT_SAFE_MILESTONES.md` — this is M9.
* `AWS_KMS_SETUP_VALIDATION_CHECKLIST.md` — AWS preflight (§5 of
  this milestone routes through there).
