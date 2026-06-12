# DeOpt V2 — Public Testnet Beta Feedback Triage Workflow

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited. Experimental.**
>
> Operator-facing playbook for processing incoming public-beta feedback. Aimed at a small operator team running the testnet beta with limited bandwidth.

This document is meant for the people running the beta, but it lives in the public docs pack on purpose — testers should be able to see how their reports are handled and what to expect.

---

## 1. Intake channels

| Channel | Source URL | Primary use | Triage cadence |
|---|---|---|---|
| GitHub issues | `{{PUBLIC_BETA_GITHUB_URL}}/issues` | Structured public bugs, integration questions, feature requests | within ~3 business days |
| Discord | `{{PUBLIC_BETA_DISCORD_URL}}` | Real-time chat, quick questions | continuous best-effort |
| Telegram | `{{TELEGRAM_INVITE_URL}}` | Real-time chat (alt to Discord) | continuous best-effort |
| Feedback form | `{{PUBLIC_BETA_FEEDBACK_URL}}` | Structured form for testers without GitHub | reviewed weekly |
| Private security inbox | GitHub Security Advisories private path | Security-impacting issues | within 1 business day |

Placeholder URLs are intentional — see [OPERATOR_PUBLIC_BETA_URLS_FILL.md](./OPERATOR_PUBLIC_BETA_URLS_FILL.md).

---

## 2. Classification

Every incoming report should be tagged with one classification + one severity.

### 2.1 Classification (one tag)

| Tag | What it means | Typical destination |
|---|---|---|
| `ux` | UX confusion, copy issues, layout / styling, small accessibility miss | Frontend repo |
| `frontend-bug` | Functional frontend bug — wrong state, broken interaction, race condition in the UI | Frontend repo |
| `backend-bug` | Backend / API behaviour — wrong response shape, wrong code, wrong envelope status | Backend repo |
| `contract-issue` | Solidity contract behaviour — unexpected revert reason, wrong event emission, accounting bug, oracle staleness | Sol repo + escalation |
| `docs-issue` | Docs inaccurate, missing, confusing, or out of date | Whichever repo holds the doc |
| `feature-request` | A thing DeOpt doesn't do today and the user wants | Roadmap doc / GitHub discussions |
| `market-maker` / `liquidity` | Request for live quotes, market-maker interest | Operator-only, manual reply |
| `security-concern` | Possible exploit path, signature forgery, vault drain, auth bypass | Private security path — see §6 |

### 2.2 Severity (one level)

| Level | Definition | Initial response SLO |
|---|---|---|
| **P0** | Blocks all testing for everyone (app won't load, backend down, all trades revert, all wallets disconnect) | Triage within 1 hour, public status update within 4 hours |
| **P1** | Serious functional bug affecting a significant subset of testers (e.g., all wallets of type X, all trades on series Y) | Triage within 1 business day |
| **P2** | Normal — affects some testers some of the time, has a workaround | Triage within 3 business days |
| **P3** | Nice-to-have, copy nits, low-impact UX papercuts | Batched weekly |

P0 and any `security-concern` always paged to the operator on-call rota (if one exists) or to the maintainer DMs (if not).

---

## 3. Reproduction requirements

Before a public bug can be "accepted" (worked on), it needs to be reproducible. A report that doesn't reproduce is NOT closed silently — it's parked with a `needs-reproduction` label and a polite reply asking for more detail.

Minimum repro for a frontend bug:
* Browser + version
* Wallet + version
* App URL / page
* Steps to reproduce (numbered)
* Screenshot OR a short screen recording

Minimum repro for a backend bug:
* Backend endpoint (path + method)
* Request body (redacted of any secret)
* Response body or error message
* Tx hash (if applicable)
* Timestamp UTC

Minimum repro for a contract issue:
* Tx hash on Base Sepolia
* Block number
* Function name + decoded args (if known)
* Expected behaviour vs actual

If a tester can supply only a screenshot — that's still useful. Park as `needs-reproduction` and ask the questions.

---

## 4. Workflow per channel

### 4.1 GitHub issue

1. Tag with one classification + one severity (see §2).
2. If repro is missing → label `needs-reproduction` + ask for the missing fields + assign back to the reporter.
3. If repro is present → label `triaged` + assign to the relevant repo's maintainer.
4. Link cross-references (e.g., "this is a manifestation of #42") if you can.
5. NEVER paste a private credential into the issue. If the reporter pasted one, ask them to rotate it, edit the comment to redact, and follow up in DM.

### 4.2 Discord / Telegram

1. Acknowledge with an emoji (eyes / waving hand) so the reporter knows they were seen.
2. If the report is reproducible right there, ask the tester to also file a GitHub issue with the template. Chat is ephemeral; GitHub is the system of record.
3. If the report is security-impacting, DM the reporter and switch to the private path immediately. Do NOT discuss specifics in the public channel.
4. Daily summary post in `#bug-reports`: "today's triaged reports".

### 4.3 Feedback form

1. Reviewed weekly by the operator.
2. Each entry triaged the same way as a GitHub issue.
3. If the entry is high-severity, escalate the same day — don't wait for the weekly review.

---

## 5. Avoiding sensitive-data intake

Public bug reports must never contain:
* private keys (any length, any encoding)
* seed phrases / mnemonics
* RPC URLs with embedded API keys
* admin bearer tokens
* full backend `.env` contents
* DATABASE_URL values
* AWS / KMS access keys
* personally identifying information beyond a public wallet address

If a report contains any of these:
1. Hide / edit the report immediately (GitHub: "Hide as off-topic" + edit comment to redact; Discord: delete message, ask user to repost without the secret).
2. DM the reporter, tell them to rotate the leaked credential.
3. Internally rotate any DeOpt-side credential that might be impacted.
4. Document the incident in the operator's private log.

This applies even if the secret was "only" a testnet RPC URL — keys belong to the tester, not to the project.

---

## 6. Escalation to security review

A `security-concern` tag is the only path that DOES NOT go through public channels.

Escalation flow:
1. Reporter opens a **private** GitHub Security Advisory (or DMs the maintainer team).
2. Operator confirms receipt within 1 business day.
3. Operator opens an internal tracking issue in the security-review packet (see `PRODUCT_FREEZE_AND_SECURITY_REANCHOR_NEXT_TASK.md`).
4. Maintainers reproduce + assess + write an internal advisory note (severity, scope, status).
5. The reporter is updated at least weekly until resolution.
6. Once a fix is shipped, the advisory may be published per CVE / Github SA convention.

A formal bug-bounty program does **not** exist yet. Acknowledgement, scoping, and (where appropriate) hall-of-fame recognition can be discussed — but no monetary reward is promised at this stage.

---

## 7. When to pause the public testnet beta

The beta is **paused** (frontend put behind a "down for maintenance" notice, banner-level announcement on Discord / Telegram) when ANY of the following becomes true:

* A confirmed P0 incident is unresolved after 24h.
* A confirmed `security-concern` of "drain / forge / bypass" class is open and not yet mitigated.
* Backend `/trading/health` is `unhealthy` for more than 1 hour with no path to recovery.
* The chain-side state is inconsistent with what the indexer or frontend show, and there is no quick reconciliation path.

Pausing is the operator's decision. Resumption requires:
* Root-cause writeup in the public docs.
* Update to [KNOWN_LIMITATIONS_AND_RISKS.md](./KNOWN_LIMITATIONS_AND_RISKS.md) if structural.
* Announcement in all channels with the same wording.

Pausing the beta is **not** a sign of failure — it's the responsible choice when the testbed isn't behaving. Communicate honestly.

---

## 8. Internal log

Operator team should keep a private (NOT in this docs pack) running log of:
* date + reporter + classification + severity + status
* triage decisions
* escalations
* postmortems for P0 incidents

This log is not public and should not be checked into the repo. The public-facing artefact is the GitHub issue tracker + Discord transcripts.

---

## 9. Closing reminder

* Be honest about scope. This is a public testnet beta — bugs are expected.
* Be transparent about pause / reset events. Trust is built through honest comms.
* Never claim audited, mainnet-ready, production, or safe for real funds. Any external communication that drifts there must be corrected directly.

---

**End of feedback triage workflow.**
