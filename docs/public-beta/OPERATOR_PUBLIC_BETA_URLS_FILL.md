# DeOpt V2 — Operator-Fill: Public Beta URLs

> **Operator-facing checklist** for swapping the `{{PLACEHOLDER}}` tokens in the public-beta docs + frontend into real, publicly-accessible URLs.
>
> **Posture:** public testnet beta. Base Sepolia only. No real funds. Unaudited. No mainnet RPC anywhere. No private credentials in any URL. No admin bearer in any link.

This doc exists because the COMMUNITY-FEEDBACK-LOOP milestone (2026-06-12) deliberately did **not** invent fake URLs. Filling them in is an operator-side action once the live channels are configured.

---

## 1. Token inventory

Every token that needs an operator-supplied real URL is listed below. The tokens appear in both the frontend link config and the public-beta docs.

| Token | Frontend file | Doc files | Expected shape | Status |
|---|---|---|---|---|
| `PUBLIC_BETA_QUICKSTART_URL` | `deopt-v2-frontend/src/lib/public-beta-links.ts` | (footer link only) | hosted URL of `BASE_SEPOLIA_QUICKSTART.md` | placeholder |
| `PUBLIC_BETA_TESTING_GUIDE_URL` | same | (footer link only) | hosted URL of `USER_TESTING_GUIDE.md` | placeholder |
| `PUBLIC_BETA_LIMITATIONS_URL` | same | (footer link only) | hosted URL of `KNOWN_LIMITATIONS_AND_RISKS.md` | placeholder |
| `PUBLIC_BETA_FEEDBACK_URL` | same | (footer + sign-failure CTA) | bug-report form URL (Tally / Google Forms / Typeform) OR GitHub Issues URL | placeholder |
| `PUBLIC_BETA_DISCORD_URL` | same | (footer + onboarding doc) | Discord invite link `https://discord.gg/...` | placeholder |
| `PUBLIC_BETA_GITHUB_URL` | same | (footer + many docs) | public GitHub repo URL `https://github.com/<org>/<repo>` | placeholder |
| `GITHUB_REPO_URL` | (none) | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md` | same as `PUBLIC_BETA_GITHUB_URL` (legacy alias) | placeholder |
| `DISCORD_INVITE_URL` | (none) | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md` | same as `PUBLIC_BETA_DISCORD_URL` (legacy alias) | placeholder |
| `TELEGRAM_INVITE_URL` | (none) | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md`, triage workflow | Telegram invite link `https://t.me/+...` | placeholder |
| `FEEDBACK_FORM_URL` | (none) | `FEEDBACK_AND_BUG_REPORTING.md`, `FAQ.md`, triage workflow | same as `PUBLIC_BETA_FEEDBACK_URL` (legacy alias) | placeholder |
| `APP_URL` | (none) | `README.md`, others | hosted URL of the public-beta app | placeholder |
| `API_BASE_URL` | (none) | `DEVELOPER_API_GUIDE.md` | publicly-callable backend API URL | placeholder |

Two token families exist because some docs were authored before the frontend link config existed. Both naming styles are kept so the docs stay readable in isolation. The operator can map them 1:1.

---

## 2. Substitution procedure

### 2.1 Frontend (`src/lib/public-beta-links.ts`)

1. Open the file.
2. For each entry in `PUBLIC_BETA_LINKS`:
   * Replace the `href` value from `"{{TOKEN}}"` to the real URL.
   * Change the `status` field from `"placeholder"` to `"live"`.
3. Run from the frontend repo:
   ```bash
   npm run typecheck
   npm run lint
   npm run build
   npx playwright test --list
   ```
4. Verify the footer renders the URL as an `<a target="_blank">` instead of a "(coming soon)" span — `isPlaceholderHref()` returns `false` for any non-`{{…}}` non-empty string.

### 2.2 Docs

In `deopt-v2-backend/docs/public-beta/`, run a careful in-place edit on each doc that references the doc-side tokens. Tokens to substitute (with the two-name aliasing kept in mind):

```bash
# inside deopt-v2-backend/docs/public-beta/, NOT a real command —
# read each doc, find the placeholder, paste the real URL.
{{GITHUB_REPO_URL}}      → https://github.com/<org>/<repo>
{{DISCORD_INVITE_URL}}   → https://discord.gg/<invite>
{{TELEGRAM_INVITE_URL}}  → https://t.me/+<invite>
{{FEEDBACK_FORM_URL}}    → https://<form-host>/<form-id>
{{APP_URL}}              → https://<your-app-host>
{{API_BASE_URL}}         → https://<your-api-host>
```

Tokens with the `PUBLIC_BETA_` prefix appear only in the frontend module — substitute them per §2.1.

### 2.3 Cross-reference

After substitution, every doc that mentions a channel should still be coherent:
* `FEEDBACK_AND_BUG_REPORTING.md` — channels table.
* `FAQ.md` — closing contact section.
* `COMMUNITY_ONBOARDING.md` — "Getting test mUSDC", "Reporting bugs".
* `FEEDBACK_TRIAGE_WORKFLOW.md` — intake channels table.
* `PUBLIC_TESTNET_BETA_ANNOUNCEMENT_DRAFT.md` — every draft uses the tokens.
* `PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` — community channels section.

---

## 3. Substitution safety rules

Before you paste a URL into a token slot, check:

* **No bearer in the URL.** `https://api.example.com/?token=abc` is a secret in a URL.
* **No RPC URL with API key.** `https://eth-sepolia.g.alchemy.com/v2/<key>` is a credential. Public RPC URLs (no key segment) are fine if they're public — but RPC URLs probably don't belong in user-facing docs at all.
* **No localhost URL announced publicly.** `http://localhost:3000` is fine in dev-local docs only; never paste it as the public-beta app URL.
* **No mainnet-related URL.** If a Basescan link references `basescan.org/...` (mainnet) instead of `sepolia.basescan.org/...` (testnet), that's a bug — the public-beta docs target the Sepolia explorer.
* **No internal operator dashboard.** Admin dashboards (`/admin/*`) are not for public links.
* **HTTPS only.** No `http://` URLs in production docs.
* **No tracking-fingerprint URLs.** No UTM tags or analytics tokens that would expose internal naming.

---

## 4. Verification after substitution

After §2 is complete:

1. **Sensitive-string scan** on the changed files:
   ```bash
   grep -nE "Bearer [A-Za-z0-9_.-]{16,}|alchemy\.com/v2/[A-Za-z0-9_-]{16,}|infura\.io/v3/[A-Za-z0-9_-]{16,}|postgres://|DATABASE_URL=|PRIVATE_KEY=|RPC_URL=https?://" $(git diff --name-only)
   ```
   Expected: zero hits.
2. **Mainnet RPC patterns**:
   ```bash
   grep -nE "mainnet\.base\.org|base-mainnet\.publicnode\.com|mainnet\.g\.alchemy\.com|api\.mainnet\.basescan" $(git diff --name-only)
   ```
   Expected: zero hits.
3. **Positive-claim drift**:
   ```bash
   grep -niE "is (audited|production[- ]ready|mainnet[- ]ready|safe for real funds)" $(git diff --name-only)
   ```
   Expected: zero hits.
4. **Footer renders live**: open the deployed app, scroll to the public-beta footer, verify every slot is a clickable anchor (not a "(coming soon)" span).
5. **Sign-failure CTA renders live**: trigger a `rejected` phase in the sign modal (Playwright fixture or local wallet rejection), verify "Report this issue" is a clickable anchor.

---

## 5. Partial substitution is fine

The frontend + docs are designed so that filling in a subset is safe. For example, you can wire up:
* Quickstart + Testing Guide + Limitations (hosted docs root is known)
* GitHub repo (it's public)

…but leave Discord + Telegram + Feedback Form as placeholders while you set those channels up. The footer + CTA degrade per-slot — `isPlaceholderHref(href)` runs per entry.

---

## 6. Closing reminder

The placeholders exist on purpose. They make absence visible. Substitute them honestly when the real channel is configured, and leave them as placeholders when it isn't. Inventing fake URLs to "complete the look" is a security regression and a trust regression.

---

**End of operator-fill checklist.**
