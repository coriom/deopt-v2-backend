# DeOpt V2 — Public Testnet Beta Announcement (FINAL DRAFT)

> **DRAFT — NOT YET PUBLISHED — DO NOT POST.**
>
> This is the final draft built from the earlier `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md`. It is **publish-ready** in copy only — it still contains `{{TOKEN}}` placeholders for URLs that are not yet live, and it requires a **separate, explicit operator approval** to actually transmit any of these messages.
>
> Required approval line for publication (verbatim, set later in a separate milestone):
>
> > "I approve DeOpt V2 public testnet beta launch publication for this run."
>
> Posture: public testnet beta. Base Sepolia (chain 84532) only. Unaudited. Experimental. No real funds. Mainnet permanently disabled. No production claims. No bug-bounty promise. No external audit completed.

---

## 0. Publish-gate checklist (operator MUST run before any post goes out)

* [ ] `PUBLIC_BETA_QUICKSTART_URL` is LIVE (not `{{PLACEHOLDER}}`).
* [ ] `PUBLIC_BETA_TESTING_GUIDE_URL` is LIVE.
* [ ] `PUBLIC_BETA_LIMITATIONS_URL` is LIVE.
* [ ] `PUBLIC_BETA_FEEDBACK_URL` is LIVE (form OR GitHub issues).
* [ ] `PUBLIC_BETA_GITHUB_URL` is LIVE.
* [x] `PUBLIC_BETA_DISCORD_URL` is LIVE — `https://discord.gg/zaEMvWuxu` (wired 2026-06-12).
* [ ] `{{APP_URL}}` is LIVE — operator-hosted, publishable URL of the deployed testnet frontend.
* [ ] `{{API_BASE_URL}}` is LIVE (only if API-only integrators are part of the announcement audience; otherwise mark NOT_REQUIRED).
* [ ] Frontend deployed: `npm run build` green from a tagged commit; deploy log archived.
* [ ] Backend `/trading/health` returns `200 ok` with `chain_id: 84532`.
* [ ] Honesty checklist (§7) passes for the channel-specific post being sent.
* [ ] Sensitive-string scan on the literal post string returns zero hits (no Bearer, no RPC URL with key, no DATABASE_URL).
* [ ] Operator + at least one secondary reviewer have signed off on the final post wording.

If any box is unchecked, **DO NOT POST**.

---

## 1. Discord — `#announcements`

Use this in the operator's Discord server, in `#announcements`, after the channel is set up per `FEEDBACK_TRIAGE_WORKFLOW.md §1`.

```
**DeOpt V2 — Public Testnet Beta is open (Base Sepolia)**

Hey everyone — DeOpt V2 is open for public testing on **Base Sepolia testnet**.

What it is:
- On-chain options protocol with two-sided EIP-712 signatures.
- Settles on chain via an operator-side executor.
- Currently on Base Sepolia (chain 84532). NOT on mainnet.

What it is NOT:
- Audited (no external review yet).
- Production-ready.
- Safe for real funds (there are none — all tokens are testnet mocks).
- A launch. This is a community preview for feedback only.

Try it:
- App: {{APP_URL}}
- Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
- User testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
- Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}

Report a bug:
- Bug form: {{PUBLIC_BETA_FEEDBACK_URL}}
- GitHub: {{PUBLIC_BETA_GITHUB_URL}}/issues

Security issues: PRIVATE path via GitHub Security Advisories on {{PUBLIC_BETA_GITHUB_URL}}. Do NOT post specifics in public.

We're a small team. We read every report. Be kind, be honest, and **never share your private key or seed phrase** — no one from the DeOpt team will ever ask you for them.

Let's break things. Thanks for testing.
```

---

## 2. X / Twitter — single post (≤ 280 chars)

```
DeOpt V2 — public testnet beta is open on Base Sepolia.

🧪 Unaudited. Experimental. No real funds.
🔗 Base Sepolia only (chain 84532). Mainnet is disabled.
🐛 Bug reports very welcome — feedback phase.

Try it: {{APP_URL}}
Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
Discord: https://discord.gg/zaEMvWuxu
```

Char check (with the live Discord and the longest plausible substitution): operator should re-measure after substitution. Trim the URL row if over budget; keep the disclaimer triplet at all costs.

---

## 3. X / Twitter — 4-tweet thread

```
1/ DeOpt V2 — public testnet beta is open on Base Sepolia. 🧪

What it is: an on-chain options protocol with two-sided EIP-712 signatures + on-chain settlement via an operator-side executor. Currently Base Sepolia (chain 84532).

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
- App: {{APP_URL}}
- Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
- Testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
- Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}
- Bug reports: {{PUBLIC_BETA_FEEDBACK_URL}}
- Discord: https://discord.gg/zaEMvWuxu
- GitHub: {{PUBLIC_BETA_GITHUB_URL}}

Thanks for testing 🙏
```

---

## 4. LinkedIn

```
We're opening the DeOpt V2 public testnet beta on Base Sepolia.

DeOpt V2 is an on-chain options protocol. Buyer and seller sign an EIP-712 typed-data trade off-chain; the matching engine settles it on chain via an operator-side executor. This release follows multiple readiness milestones (M-P1 through M-P5), a docs-only public-beta pack, a frontend product polish, and a security-review preparation packet.

What this is:
• A public, community-preview testnet beta running on Base Sepolia (chain 84532).
• Unaudited. Experimental. Feedback phase.
• An invitation to test, integrate, and report what breaks.

What this is NOT:
• A mainnet launch.
• Audited.
• Safe for real funds.
• Institutional-grade.

We are a small team and we read every report. Security disclosures go through a private GitHub Security Advisories path. Public bugs go to the feedback form, GitHub Issues, Discord, or Telegram.

If you'd like to test:
• App: {{APP_URL}}
• Quickstart: {{PUBLIC_BETA_QUICKSTART_URL}}
• Testing guide: {{PUBLIC_BETA_TESTING_GUIDE_URL}}
• Known limitations: {{PUBLIC_BETA_LIMITATIONS_URL}}
• Bug reports: {{PUBLIC_BETA_FEEDBACK_URL}}
• Discord: https://discord.gg/zaEMvWuxu
• GitHub: {{PUBLIC_BETA_GITHUB_URL}}

We deliberately are not over-promising. Mainnet activation gates on a security review and an external audit. Until then: please treat this as a sandbox, not a product.

Thanks to everyone helping us test. Honest feedback >> hype.
```

---

## 5. GitHub README banner

Markdown snippet to prepend to the top-level `README.md` of the public repo. Safe to land as a PR before any external post goes out — it sets expectations for visitors.

```markdown
> ⚠ **Public testnet beta — UNAUDITED — experimental — Base Sepolia only.**
> DeOpt V2 is currently a **public testnet beta** on Base Sepolia (chain id 84532).
> It is **not audited**, **not mainnet-ready**, and **not safe for real funds**.
> Mainnet is permanently disabled in the UI; the protocol does not deploy contracts to Base mainnet.
> See `docs/public-beta/` for the full public docs pack, including the
> [Quickstart]({{PUBLIC_BETA_QUICKSTART_URL}}),
> [Known Limitations]({{PUBLIC_BETA_LIMITATIONS_URL}}),
> [Feedback channels]({{PUBLIC_BETA_FEEDBACK_URL}}), and
> [Discord](https://discord.gg/zaEMvWuxu).
```

---

## 6. Pause / rollback announcement template

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

## 7. Honesty checklist (run before posting each channel)

For every draft above, before posting confirm:

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
- [ ] No RPC URL with an embedded API key, no admin bearer token, no DATABASE_URL, no `.env` content in the post string.
- [ ] All `{{TOKEN}}` placeholders have been substituted with the live URL OR the line referencing the placeholder has been removed.

If any box fails, fix the copy. Honest framing > momentum.

---

## 8. Sensitive-string post-check (run on the rendered post string)

Even after substitution, run a final scan against the literal text that's about to be posted:

```bash
# Run on the rendered post copied to a temp file before sending.
grep -nE "Bearer [A-Za-z0-9_.-]{16,}|alchemy\.com/v2/[A-Za-z0-9_-]{16,}|infura\.io/v3/[A-Za-z0-9_-]{16,}|postgres://|DATABASE_URL=|PRIVATE_KEY=" /tmp/post-to-send.txt
```

Expected: zero hits. Any hit = do not post; rotate credential, redact, re-scan.

---

## 9. Versioning + sign-off

* This draft snapshot lives at `docs/public-beta/PUBLIC_TESTNET_BETA_ANNOUNCEMENT_FINAL_DRAFT.md`.
* If the operator amends it before publication, the amended version supersedes this snapshot — but the original `{{TOKEN}}` discipline + the honesty checklist must still hold.
* Sign-off lives in the operator's private notes; the public log is the post itself.

---

**End of public testnet beta announcement (FINAL DRAFT). DO NOT POST until §0 publish-gate checklist passes and a separate operator approval line is consumed.**
