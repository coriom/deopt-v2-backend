# E2E Sepolia — Operator Input Private Template

**Date:** 2026-06-10
**Audience:** operator. Copy this template to a PRIVATE / UNTRACKED
location (NOT this checked-in file). Fill in the values there.
**Posture:** **template only.** This checked-in copy contains
placeholders. The PRIVATE filled-in copy MUST live outside the
tracked tree (suggested: `~/DEOPT/operator-private/`).

> **Hard rules:**
> * NEVER commit a populated copy.
> * NEVER paste a real RPC URL into a tracked file.
> * NEVER paste a private key anywhere — not even the private copy.
> * NEVER paste an AWS account ID / KMS key ID / KMS ARN.
> * The `.gitignore` files already cover `operator-private/`,
>   `*.private.md`, `*.private.env`, `.env.sepolia*`. Stay inside
>   those patterns.

## 1. Suggested private filename

`~/DEOPT/operator-private/sepolia.inputs.private.md`

(`operator-private/` is covered by every repo's `.gitignore`.)

## 2. Required private values (fill in the private copy)

```ini
# === Sepolia RPC ===
EXECUTION_RPC_URL=<operator-supplied; Sepolia HTTPS RPC>

# === Operator-side addresses (NOT yet in checked-in docs) ===
OPTION_MARGIN_ENGINE_LENS_ADDRESS=<0x40-hex-address>
COLLATERAL_TOKEN=<0x40-hex-address; the ERC20 collateral token on Sepolia>
PROTOCOL_FEE_VAULT=<optional; only if operator wants confirmation>
FEES_MANAGER_V2=<optional; from deploy notes>

# === Optional pre-selected series ===
ACTIVE_OPTION_SERIES_ID=<operator-chosen series id, e.g. backend store id>
```

## 3. Already-public values (NO need to put in private copy)

These are already documented in `~/DEOPT/TESTNET_RUNBOOK.md` and the
M-P5 docs — they are public addresses:

```ini
CHAIN_ID=84532
NETWORK_NAME=base-sepolia
OPTION_PRODUCT_REGISTRY=0x3d52b033fab00ed6104dd3bc0a715f8648344eca
OPTION_MATCHING_ENGINE=0xf2D1D85cD363Be3bc160d14883C80e7C2c4F420b
OPTION_MARGIN_ENGINE=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_MARGIN_ENGINE_ADDRESS=0x287Cef479be5889eEfCa847F9e73C860898f48Cc
OPTION_COLLATERAL_VAULT=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_COLLATERAL_VAULT_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_COLLATERAL_VAULT_VIEWS_ADDRESS=0x00340C360353a5AB784c5Bc5c44322A6AF0625D3
OPTION_ORACLE_ROUTER_ADDRESS=0xb416406f200b2ef3d7a86a5d5877ed41d9b1a581
EXECUTOR_ADDRESS=0xc35F7A8A103A9A4464adfaa76B9B514093D23C27
BUYER_ADDRESS=0xc0A76c2A6c6b70C0B065A05E64417886416cc976
SELLER_ADDRESS=0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3
```

## 4. How to use the private file

```bash
# In a NEW operator shell:
set -a; source ~/DEOPT/operator-private/sepolia.inputs.private.md; set +a
# (this file uses key=value lines; bash source works)

# Then run the M-P5-RO confirmation milestone again:
cd ~/DEOPT
# the harness picks up the env vars; no .env edit needed.
```

If sourcing a `.md` file is uncomfortable, use a `.private.env`
variant:

```bash
cp ~/DEOPT/operator-private/sepolia.inputs.private.md \
   ~/DEOPT/operator-private/sepolia.inputs.private.env
# then strip markdown formatting
set -a; source ~/DEOPT/operator-private/sepolia.inputs.private.env; set +a
```

## 5. What the next milestone consumes

`SEPOLIA-OPERATOR-INPUT-PROVISIONING-AND-READONLY-CHECKS` (this
milestone) re-runs and:

1. Detects env vars set (presence only).
2. Runs the BS-2 / BS-3 / BS-4 / BS-5 read-only `cast` calls.
3. Updates the checklist with CONFIRMED status flags only.
4. Decides whether the live approval gate is READY.

## 6. Cross-links

* `E2E_SEPOLIA_OPERATOR_INPUT_TEMPLATE.md` (public-safe template, M-P5-FIXES)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATION_LOG.md` (cast-call playbook)
* `E2E_SEPOLIA_READ_ONLY_CONFIRMATIONS_RESULT.md` (M-P5-RO)
* `E2E_SEPOLIA_OPERATOR_INPUT_PROVISIONING_RESULT.md` (this milestone)
* `~/DEOPT/TESTNET_RUNBOOK.md`

**End of private template.**
