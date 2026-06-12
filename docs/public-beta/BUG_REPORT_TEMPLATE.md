# DeOpt V2 — Public Testnet Beta Bug Report Template

> **Public testnet beta. Base Sepolia only. No real funds. Unaudited. Experimental.**
>
> Copy the template block below into your GitHub issue, your Discord message, or the feedback form. Fill in as much as you can — partial reports are still useful.

This template is for **public, non-security** bugs. If you think you've found a security-impacting issue (e.g., a way to drain the vault, forge signatures, bypass authorisation, or exfiltrate operator state), use the private security disclosure path described in [FEEDBACK_AND_BUG_REPORTING.md §7](./FEEDBACK_AND_BUG_REPORTING.md) — do **NOT** post it in a public channel.

---

## 1. Safety rules — read first

Before you post anything:

* **NEVER share a private key.** Not the full key. Not a partial key. Not in a screenshot, video, console log, or text.
* **NEVER share a seed phrase / mnemonic.** Twelve, eighteen, or twenty-four words — none of them, ever.
* **NEVER share an RPC URL with an embedded API key.** URLs like `https://eth-sepolia.g.alchemy.com/v2/<your-key>` contain a credential. Redact the key segment (`/v2/REDACTED`) before posting.
* **NEVER share an admin bearer token.** If you happen to see one in a curl example, treat it like a credential — DM the maintainers privately instead of posting.
* **NEVER share the contents of a backend `.env`, `secrets.json`, AWS credential file, or anything similar.**
* **DO share** your wallet's **public address** (`0x…`), the tx hash, the page URL, the browser version, and the operating system. These are not secrets.

If you accidentally posted a secret, **rotate it immediately** and tell the maintainers privately so they can scrub the public log.

---

## 2. Bug report template

Paste this whole block into your report and fill in the fields. Lines you can't fill in: leave them blank or write `n/a`.

```
### Issue title

(One short sentence. Example: "Sign button stays disabled after switching networks.")

### Test scenario

(What were you trying to do? Example: "Buy 1 contract of the ETH-USDC Call at 3000 strike.")

### Severity guess

(P0 / P1 / P2 / P3 — see FEEDBACK_TRIAGE_WORKFLOW.md. Your guess is helpful but not binding.)

### Environment

- Wallet public address (Base Sepolia): 0x...
- Network: Base Sepolia
- Chain id seen by the app: 84532
- Timestamp (UTC): YYYY-MM-DDThh:mm:ssZ
- Tx hash (if any): 0x...
- Were you on Base Sepolia at the time? yes
- Were real funds involved? no  ← expected answer is no; this is testnet only
- Browser + version: e.g. Chrome 128 / Firefox 130 / Safari 18
- Wallet provider + version: e.g. MetaMask 12.x / Rabby 2.x / Coinbase Wallet 30.x
- OS: e.g. macOS 15 / Windows 11 / Linux Ubuntu 24
- App URL / page: e.g. /markets/0x… or /transactions/<intent_id>

### Steps to reproduce

1. ...
2. ...
3. ...

### Expected behavior

(What you thought should happen.)

### Actual behavior

(What actually happened, including any inline error text the app showed.)

### Console errors (if safe to share)

(Open browser DevTools → Console. Copy any RED errors that look related. **Redact** any RPC URL key, bearer token, or anything that looks like a secret before pasting.)

### Screenshots / short video

(Attach. Crop or blur anything that might contain a secret.)

### Anything else

(Anything you noticed, like "this only happens after I switch networks twice" or "intermittent — happens about 1 in 3 attempts".)
```

---

## 3. What we'll use it for

* Reproducing the bug locally.
* Filing a tracking issue (linked back to your report).
* Updating the [KNOWN_LIMITATIONS_AND_RISKS.md](./KNOWN_LIMITATIONS_AND_RISKS.md) doc if the issue is structural rather than a one-off.

We don't promise a fix on every report. We do promise every report is read.

---

## 4. Reporting checklist

Before you hit submit:

- [ ] No private key in the report.
- [ ] No seed phrase / mnemonic in the report.
- [ ] No RPC URL with an embedded API key.
- [ ] No admin bearer token.
- [ ] No `.env` contents.
- [ ] Tx hash + intent id + page URL included (if applicable).
- [ ] Wallet **public** address included (not the private key).
- [ ] Browser + wallet + OS + chain id filled in.
- [ ] Screenshots cropped / blurred of anything sensitive.
- [ ] Posted in the **right** channel — public bug report goes to the public channels; security disclosure goes to the private path.

---

**End of bug report template.**
