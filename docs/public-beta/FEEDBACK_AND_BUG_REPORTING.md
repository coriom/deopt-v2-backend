# DeOpt V2 — Feedback and Bug Reporting

> **Testnet only. No real funds. Unaudited.** We welcome your bug reports, UX feedback, and integration questions.

---

## 1. Channels

The DeOpt V2 public-beta team accepts feedback through the following channels:

| Channel | URL | Best for |
|---|---|---|
| GitHub issues | `{{ GITHUB_REPO_URL }}/issues` | Reproducible bugs, feature requests, integration questions, API specifics. |
| Discord | `{{ DISCORD_INVITE_URL }}` | Real-time chat, quick questions, community discussion. |
| Telegram | `{{ TELEGRAM_INVITE_URL }}` | Real-time chat (alt to Discord). |
| Feedback form | `{{ FEEDBACK_FORM_URL }}` | Structured bug reports if you'd rather not open a GitHub account. |

> Placeholder URLs above are intentional. The operator fills them in before the beta is publicly announced. If you're reading this and the URLs are still placeholders, the beta hasn't launched publicly yet. See [OPERATOR_PUBLIC_BETA_URLS_FILL.md](./OPERATOR_PUBLIC_BETA_URLS_FILL.md) for the substitution checklist.

---

## 2. Bug report template

> **Quick path:** copy from [BUG_REPORT_TEMPLATE.md](./BUG_REPORT_TEMPLATE.md) — that doc has a fuller safety checklist (no private keys, no seed phrases, no RPC URLs with embedded keys, no admin bearer tokens, no `.env` contents) and a more detailed environment table.
>
> The compact template below remains here for reference.

Please include as many of these fields as you can. The more we have, the faster we can reproduce.

```
### Summary

(One sentence describing what went wrong.)

### Steps to reproduce

1. ...
2. ...
3. ...

### Expected behavior

(What you thought should happen.)

### Actual behavior

(What actually happened, including any error messages.)

### Environment

- Wallet address (Base Sepolia): 0x...
- Network: Base Sepolia (chain id 84532)
- Tx hash (if any): 0x...
- Timestamp (UTC): YYYY-MM-DDThh:mm:ssZ
- Browser: e.g. Chrome 128 / Firefox 130 / Safari 18
- Wallet: e.g. MetaMask 12.x / Rabby 2.x / Coinbase Wallet 30.x
- OS: e.g. macOS 15 / Windows 11 / Linux Ubuntu 24
- App URL: <which page were you on>

### Screenshots / video

(Attach if applicable.)

### Anything else

(Logs from the wallet, network tab errors, suspected root cause, etc.)
```

You can copy that block into a GitHub issue, a Discord message, or the feedback form. We will accept any subset.

---

## 3. What we want feedback on

* **Wallet UX.** Did the wallet's typed-data signing prompt show the trade fields clearly? Did it show garbled hex or proper labels?
* **Network detection.** Did the app correctly detect when you were on the wrong network? Did the switch flow work in your wallet?
* **Lifecycle UI.** Did the trade-status page update in real time, or did you have to refresh? Were the status labels clear?
* **Error messages.** Were error messages clear about what went wrong and what to do? "Stale oracle" warnings — did the UI explain that they're expected on testnet?
* **API ergonomics** (developers). Was the OpenAPI spec accurate? Were the request / response shapes intuitive? Anything missing?
* **Documentation.** Was this docs pack clear? Did anything confuse you?

---

## 4. What we'd LOVE to know about

* **Reproducible reverts.** Any executeTrade tx that reverted unexpectedly — please share the tx hash.
* **Confusing UX.** If something felt wrong, even if you can't articulate why, tell us. "Confused at step X" is a useful report.
* **Wallet incompatibilities.** If a specific wallet doesn't work, please let us know which one, what version, and what failed.
* **Integration friction.** If you're integrating via the API and hit something weird, the OpenAPI spec is supposed to be canonical — if it isn't, that's a high-priority bug.

---

## 5. What NOT to do

### NEVER share private credentials

* **No private keys.** Not as a string, not in a screenshot, not in a video, not even partial.
* **No seed phrases / mnemonics.** Same — never.
* **No backend admin bearer tokens.** If you somehow saw one (e.g., leaked in a curl example), do NOT post it; instead, tell us privately so we can rotate.

> The DeOpt team will NEVER ask you for a private key, seed phrase, or password. If anyone in the community asks you for these and claims to be from DeOpt, they are an impostor — block them and report them.

### NEVER spam

* One bug report per issue. If you have five bugs, that's five separate reports (in GitHub) or five separate messages (in chat).
* No promotional content. Discussion of other projects is fine; promotional spam is not.

### NEVER claim production use

* This is a testnet beta. Do not claim DeOpt is "live", "audited", "mainnet-ready", or "safe for real funds" in any external communication. We will be very direct in correcting that wherever we see it.

---

## 6. Response expectations

* **GitHub issues.** Triaged within ~3 business days. High-severity bugs may get attention sooner.
* **Chat (Discord / Telegram).** Best-effort. Operator team is small.
* **Feedback form.** Reviewed weekly.

We can't promise a fix or even a response on every report — but every report is read.

---

## 7. Security disclosures

If you've found something that looks like a serious security issue (e.g., a way to drain the vault, forge signatures, or bypass authorization), **please do NOT post it in public channels**. Instead:

* Open a PRIVATE GitHub security advisory at `{{ GITHUB_REPO_URL }}/security/advisories/new`, OR
* DM the maintainer team via the Discord / Telegram channels listed above.

We treat these reports seriously. A formal bug-bounty program is not active yet, but acknowledgement, scoping, and (where appropriate) hall-of-fame recognition can be discussed.

---

## 8. Privacy

* We don't collect personal data via the docs pack itself.
* The frontend may log anonymized analytics (wallet address truncated to first 6 chars + chain id + timestamp); this is to size the user base.
* The feedback form will collect whatever you put into it. Don't put anything in there you wouldn't be comfortable sharing.
* Wallet addresses are public; assume they will be visible to the team and to anyone reading public bug reports.

---

## 9. Closing reminder

* **Testnet only.** No real funds at risk.
* **Be patient.** The team is small; the beta is experimental.
* **Be kind.** Bug reports are welcome; rudeness is not.

Thank you for testing DeOpt V2. Your feedback is what makes a beta worth running.

---

## 10. See also

* [BUG_REPORT_TEMPLATE.md](./BUG_REPORT_TEMPLATE.md) — full-fat bug report template with safety checklist.
* [FEEDBACK_TRIAGE_WORKFLOW.md](./FEEDBACK_TRIAGE_WORKFLOW.md) — operator-side triage classification + severity + escalation.
* [COMMUNITY_ONBOARDING.md](./COMMUNITY_ONBOARDING.md) — onboarding for external testers.
* [PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md](./PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md) — operator launch checklist.
* [PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md](./PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md) — announcement copy drafts (incl. pause / rollback template).
* [OPERATOR_PUBLIC_BETA_URLS_FILL.md](./OPERATOR_PUBLIC_BETA_URLS_FILL.md) — placeholder-URL substitution checklist.

---

**End of feedback and bug reporting.**
