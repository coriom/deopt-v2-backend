# OPERATOR-PUBLIC-BETA-URLS-FILL — RERUN — Next Task Brief

**Date written:** 2026-06-13
**Origin:** `FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`
**Supersedes verdict block of:** `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` (the original brief stays valid; this rerun records the new context)

**Target:** rerun `OPERATOR-PUBLIC-BETA-URLS-FILL` once the operator has stood up the public-beta frontend at `<APP_URL>`. The URLs to substitute are now narrowed: the only remaining token whose value is gated on operator hosting is `{{APP_URL}}`. The other live tokens are already wired (see `OPERATOR_PUBLIC_BETA_URLS_REMAINING_ACTIONS.md`).

**Posture:** **docs + frontend link-config string substitution only. NEVER mainnet. NEVER chain transactions. NEVER backend `.env` edit. NEVER private key handling. NEVER add admin / bearer / RPC-with-key URLs.**

---

## 1. Literal operator approval line (REQUIRED, VERBATIM)

> "I approve DeOpt V2 operator public beta URLs fill for this run."

(Same line as the original brief — reuse is intentional.)

---

## 2. Hard precondition

* `<APP_URL>` is live, HTTPS, and reachable. Verify with `curl -I <APP_URL>` → 200.
* `<APP_URL>/docs/quickstart`, `<APP_URL>/docs/testing-guide`, `<APP_URL>/docs/limitations`, `<APP_URL>/feedback` all return 200.
* `<APP_URL>/trading` smoke-test from `FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md §3` passes.

If any of the above fail, STOP. Fix the deployment first, then re-attempt this rerun.

---

## 3. Substitutions to perform

The operator should set the following exact values (no invention; the operator MUST supply `<APP_URL>` from the actual deployment):

```
PUBLIC_BETA_APP_URL=<APP_URL>
PUBLIC_BETA_DOCS_URL=<APP_URL>/docs
PUBLIC_BETA_QUICKSTART_URL=<APP_URL>/docs/quickstart
PUBLIC_BETA_TESTING_GUIDE_URL=<APP_URL>/docs/testing-guide
PUBLIC_BETA_LIMITATIONS_URL=<APP_URL>/docs/limitations
PUBLIC_BETA_FEEDBACK_URL=<APP_URL>/feedback
PUBLIC_BETA_DISCORD_URL=https://discord.gg/zaEMvWuxu
PUBLIC_BETA_GITHUB_URL=https://github.com/DeOpt
```

Note: `PUBLIC_BETA_QUICKSTART_URL` / `TESTING_GUIDE_URL` / `LIMITATIONS_URL` / `FEEDBACK_URL` are ALREADY live as internal frontend routes; the rerun is updating the **doc-side** token expansion so the public-beta docs and announcement drafts point at the actually-hosted URL rather than the relative path.

Frontend `src/lib/public-beta-links.ts` does NOT need any href change here — its slots are already either internal routes (which work automatically once the app is hosted) or external (Discord / GitHub).

---

## 4. Out of scope

* Inventing URLs. The operator must supply `<APP_URL>`.
* `{{ API_BASE_URL }}` doc-side token (NOT_REQUIRED_FOR_LAUNCH; frontend bundles backend URL at build time).
* Status page URL (NOT_REQUIRED_FOR_LAUNCH).
* Any chain transaction.
* Mainnet anything.
* Publishing the announcement.

---

## 5. Acceptance criteria

* All doc-side `{{APP_URL}}` tokens in `deopt-v2-backend/docs/public-beta/**` substituted with `<APP_URL>`.
* Announcement drafts updated similarly (still UNPUBLISHED — only the template is filled).
* `git diff --check` clean.
* Sensitive-string scan zero hits.
* Mainnet RPC pattern scan zero hits.
* Positive-claim drift scan zero true hits.
* No source change outside the docs + (if needed) frontend link config.

---

## 6. Follow-up after this rerun

Re-run `PUBLIC-TESTNET-BETA-LAUNCH-PREFLIGHT` per `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md`. With `{{APP_URL}}` live, the verdict should flip to **READY**. That preflight will then generate `PUBLIC_TESTNET_BETA_LAUNCH_NEXT_TASK.md` with the publication approval line — which is a separate, explicit, operator-controlled action.

---

## 7. Cross-links

* `docs/OPERATOR_PUBLIC_BETA_URLS_FILL_NEXT_TASK.md` (original brief)
* `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_PREFLIGHT_RESULT.md`
* `docs/FRONTEND_PUBLIC_TESTNET_DEPLOY_OPERATOR_CHECKLIST.md`
* `docs/PUBLIC_TESTNET_BETA_LAUNCH_PREFLIGHT_RERUN_NEXT_TASK.md`
* `docs/public-beta/PUBLIC_TESTNET_BETA_LAUNCH_CHECKLIST.md` §0
* `~/DEOPT/RUN_STATE.md`

**End of operator public beta URLs fill RERUN next-task brief.**
