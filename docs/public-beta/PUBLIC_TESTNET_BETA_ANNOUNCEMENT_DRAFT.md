# DeOpt V2 — Public Testnet Beta Announcement Drafts

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited. Experimental.** Drafts for the announcement copy. Pick the one matching your channel, fill in the placeholder URLs, send.

Every draft below has been written to:
* lead with "public testnet beta",
* never say "audited",
* never say "mainnet-ready",
* never imply real-funds safety,
* never use "institutional", "production", or "live launch" language,
* explicitly ask for feedback,
* explicitly mark the experimental nature.

Operator should sanity-check each draft against the [PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md](./PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md) before posting.

Replace every `{{PLACEHOLDER}}` token before posting. The replacements are tracked in [OPERATOR_PUBLIC_BETA_URLS_FILL.md](./OPERATOR_PUBLIC_BETA_URLS_FILL.md).

---

## 1. Discord — `#announcements`

```
[Public testnet beta — Base Sepolia — unaudited]

Hey everyone — DeOpt V2 is open for public testing on **Base Sepolia testnet**.

What it is:
- On-chain options protocol.
- EIP-712 two-sided signature flow.
- Currently on Base Sepolia (chain 84532). NOT on mainnet.

What it is NOT:
- Audited. (No external review yet.)
- Production-ready.
- Safe for real funds. (There are none. All tokens are testnet mocks.)
- A launch. This is a **community preview** for feedback only.

Want to try it?
1. Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
2. User testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
3. Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}

Want to report a bug?
- Bug template: {{PUBLIC_BETA_FEEDBACK_URL}}
- GitHub: {{PUBLIC_BETA_GITHUB_URL}}/issues

Security issues: PRIVATE path via GitHub Security Advisories. Do NOT post in public.

We're a small team. We read every report. Be kind, be honest, and **never share your private key or seed phrase** — no one from the DeOpt team will ever ask you for them.

Let's break things. Thanks for testing.
```

---

## 2. X / Twitter (single post)

```
DeOpt V2 — public testnet beta is open on Base Sepolia.

🧪 Unaudited. Experimental. No real funds.
🔗 Base Sepolia only (chain 84532). Mainnet is disabled.
🐛 Bug reports very welcome — feedback phase.

Try it: {{PUBLIC_BETA_QUICKSTART_URL}}
GitHub: {{PUBLIC_BETA_GITHUB_URL}}
Discord: {{PUBLIC_BETA_DISCORD_URL}}

Not audited. Not mainnet-ready. Don't trade real value.
```

Char budget: < 280 for X. If over budget, trim the closing line. NEVER trim the "Unaudited / Experimental / No real funds" line — that's the load-bearing disclaimer.

---

## 3. X / Twitter (thread, 4 tweets)

```
1/ DeOpt V2 — public testnet beta is open on Base Sepolia. 🧪

What it is: an on-chain options protocol with EIP-712 two-sided signatures + on-chain settlement. Currently Base Sepolia (chain 84532).

Not audited. Not mainnet-ready. No real funds.

Try it 👇

2/ The protocol:
- Buyer + seller sign EIP-712 typed data off-chain.
- Executor service broadcasts `executeTrade(buyer_sig, seller_sig, …)`.
- Margin engine + collateral vault settle on chain.
- Indexer reconciles events back into a public API.

3/ This is a public testnet beta. Things will break, addresses may change, the DB may be reset. That's expected — we want feedback.

NOT audited. NOT safe for real funds. Mainnet is disabled in the UI.

4/ Get started:
- Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
- Testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
- Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}
- Bug reports: {{PUBLIC_BETA_FEEDBACK_URL}}
- Discord: {{PUBLIC_BETA_DISCORD_URL}}

Thanks for testing 🙏
```

---

## 4. LinkedIn

```
We're opening the DeOpt V2 public testnet beta on Base Sepolia.

DeOpt V2 is an on-chain options protocol. Buyer and seller sign an EIP-712 typed-data trade off-chain; the matching engine settles it on chain via an executor service. It is the result of multiple readiness milestones (M-P1 through M-P5) and a docs-only public-beta pack.

What this is:
• A public, community-preview testnet beta running on Base Sepolia (chain 84532).
• Unaudited. Experimental. Feedback-phase.
• An invitation to test, integrate, and report what breaks.

What this is NOT:
• A mainnet launch.
• Audited.
• Safe for real funds.
• Institutional-grade.

We are a small team and we read every report. Security disclosures go through a private GitHub Security Advisories path. Public bugs go to the feedback form, GitHub Issues, Discord, or Telegram.

If you'd like to test:
• Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
• Testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
• Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}
• Bug reports: {{PUBLIC_BETA_FEEDBACK_URL}}
• GitHub: {{PUBLIC_BETA_GITHUB_URL}}

We deliberately are not over-promising. Mainnet activation gates on a security review and an external audit. Until then: please treat this as a sandbox, not a product.

Thanks to everyone helping us test. Honest feedback >> hype.
```

---

## 5. GitHub repo README banner

A short banner suitable for prepending to the top-level `README.md` of the public repo:

```markdown
> ⚠ **Public testnet beta — UNAUDITED — experimental — Base Sepolia only.**
> DeOpt V2 is currently a **public testnet beta** on Base Sepolia (chain id 84532).
> It is **not audited**, **not mainnet-ready**, and **not safe for real funds**.
> Mainnet is disabled in the UI; the protocol does not deploy contracts to Base mainnet.
> See `docs/public-beta/` for the full public docs pack, including the
> [Quickstart]({{PUBLIC_BETA_QUICKSTART_URL}}),
> [Known Limitations]({{PUBLIC_BETA_LIMITATIONS_URL}}),
> and [Feedback channels]({{PUBLIC_BETA_FEEDBACK_URL}}).
```

---

## 6. Email to early testers (optional)

```
Subject: DeOpt V2 — public testnet beta is open (Base Sepolia)

Hi {{TESTER_NAME}},

Sharing this with people who signed up early. DeOpt V2 is now open for public testing on **Base Sepolia testnet**. No real funds, no audit, no SLA, no launch — this is a community preview to gather feedback.

If you have ~30 minutes, we'd love to know how the trade flow feels on your wallet. The Quickstart is here: {{PUBLIC_BETA_QUICKSTART_URL}}.

A few things to keep in mind:
- Base Sepolia only (chain 84532). Mainnet is disabled.
- Not audited.
- All tokens are testnet mocks; mUSDC has no real-world value.
- Bugs are expected. Please report them via the template: {{PUBLIC_BETA_FEEDBACK_URL}}.

If you find anything that looks security-impacting, please DO NOT post it in a public channel. Open a private GitHub Security Advisory on {{PUBLIC_BETA_GITHUB_URL}} instead.

Thanks for trying it.

— DeOpt team
```

---

## 7. Pause / rollback announcement (in case of a P0 incident)

Keep this ready. If you need it, edit the brackets and post in the same channels as the launch announcement.

```
[Beta paused — public testnet beta]

We've paused the DeOpt V2 public testnet beta as of {{TIMESTAMP_UTC}} after a {{P0|P1}} incident affecting {{SCOPE}}.

Status:
- Frontend is showing a "down for maintenance" notice.
- Backend is {{ok | degraded | unhealthy}}.
- Contracts are unchanged on chain.
- No real-funds exposure (this remains a testnet beta).

Cause (initial assessment): {{ONE_OR_TWO_SENTENCES}}.

What we're doing:
- {{ACTION_1}}
- {{ACTION_2}}

ETA to resume: {{HOURS | "not yet known"}}.

We'll post the next update within {{INTERVAL}}. As always, never share your private key or seed phrase, and do NOT use the app while it's paused.

— DeOpt team
```

---

## 8. Honesty checklist (run before posting)

For every announcement above, before posting confirm:

- [ ] The word "audited" appears only with a NEGATIVE qualifier ("not audited", "no audit", "unaudited"). Never as a positive claim.
- [ ] The word "mainnet" appears only as a NEGATIVE ("mainnet is disabled", "not mainnet-ready", "not on mainnet"). Never as a positive launch.
- [ ] The phrase "real funds" appears only as a NEGATIVE ("no real funds", "not safe for real funds"). Never as a positive.
- [ ] The phrase "public testnet beta" appears at least once.
- [ ] The phrase "Base Sepolia" appears at least once.
- [ ] There is a feedback channel link.
- [ ] There is a security disclosure mention (or it's in the linked docs).
- [ ] No "we're live" / "we've launched" / "open for production" / "institutional-grade" wording.
- [ ] No price-prediction / financial-advice wording.
- [ ] No bug-bounty promise. (There is no bounty yet.)

If any box fails, fix the copy. Honest framing > momentum.

---

**End of public testnet beta announcement drafts.**
