# NEXT_TASK.md — MM Permissions V1A: Allowlist and Capability Gating

## Context

Production MM Auth V1A is implemented.

Current state:

- WebTransport MM sessions can authenticate via wallet challenge.
- Session is bound to mm_account.
- When auth is required, protected messages require authenticated sessions.
- Payload account/mm_account must match the authenticated session account.

Missing layer:

Authentication proves identity, but does not decide whether this MM is allowed to quote/trade a given product.

## Goal

Implement MM Permissions V1A.

Add an allowlist/capability layer for authenticated MM accounts.

The backend must enforce:

- only enabled MM accounts can use protected MM actions when permission enforcement is enabled
- MM accounts can have separate capabilities:
  - perp RFQ quotes
  - option RFQ quotes
  - perp order submission
  - option order submission if applicable later
- optional market-level permissions for perps
- optional option-series permissions for options
- disabled/dev mode remains available

## Non-Goals

Do not implement:

- frontend permissions UI
- admin write endpoints
- automatic MM approval
- scoring/ranking
- incentives/rebates
- on-chain allowlist
- Solidity changes
- deployments
- OAuth/API keys

## Safety Rules

Do not:

- modify Solidity
- deploy contracts
- enable real broadcast by default
- expose private keys
- require live RPC/Postgres/WebTransport/private keys for normal cargo test
- break disabled/dev auth mode
- commit
- push

## Config

Add:

```env
MM_PERMISSIONS_ENABLED=false
MM_PERMISSIONS_REQUIRE_PERSISTENCE=true

Behavior:

if disabled: current behavior preserved
if enabled:
protected MM actions require account to be enabled in MM permissions store
missing account is rejected
disabled account is rejected
capability mismatch is rejected

If persistence is required and unavailable, startup fails clearly.

Database

Add migration:

migrations/0017_mm_permissions.sql

Suggested tables:

CREATE TABLE mm_accounts (
    mm_account TEXT PRIMARY KEY,
    enabled BOOLEAN NOT NULL,
    label TEXT NULL,
    can_submit_perp_orders BOOLEAN NOT NULL DEFAULT FALSE,
    can_quote_perp_rfq BOOLEAN NOT NULL DEFAULT FALSE,
    can_quote_option_rfq BOOLEAN NOT NULL DEFAULT FALSE,
    can_submit_option_orders BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE TABLE mm_market_permissions (
    id TEXT PRIMARY KEY,
    mm_account TEXT NOT NULL REFERENCES mm_accounts(mm_account),
    market_id BIGINT NULL,
    option_series_id TEXT NULL,
    enabled BOOLEAN NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE INDEX idx_mm_accounts_enabled ON mm_accounts(enabled);
CREATE INDEX idx_mm_market_permissions_account ON mm_market_permissions(lower(mm_account));
CREATE INDEX idx_mm_market_permissions_market ON mm_market_permissions(market_id);
CREATE INDEX idx_mm_market_permissions_option_series ON mm_market_permissions(option_series_id);

Interpretation:

market_id IS NULL AND option_series_id IS NULL can mean global permission if you choose.
Or use explicit capability-only permissions in mm_accounts.
Keep semantics simple and document.
Store / Service

Add:

src/mm/permissions.rs

or extend MM service cleanly.

Expose functions:

check_mm_enabled(account)
check_can_quote_perp_rfq(account, market_id)
check_can_quote_option_rfq(account, option_series_id)
check_can_submit_perp_order(account, market_id)
Enforcement Points

When MM_PERMISSIONS_ENABLED=true, enforce:

Perp RFQ quote

rfq_quote.mm_account must:

be authenticated session account if auth required
exist in mm_accounts
enabled=true
can_quote_perp_rfq=true
allowed for market_id if market-level permission is configured
Option RFQ quote

option_rfq_quote.mm_account must:

be authenticated session account if auth required
exist in mm_accounts
enabled=true
can_quote_option_rfq=true
allowed for option_series_id if series-level permission is configured
Perp submit_order / quote_replace

If these paths exist through MM gateway:

account must be enabled
can_submit_perp_orders=true
allowed for market_id if configured

Do not overbuild option order MM path if not integrated yet.

Admin Read-only Visibility

Extend existing admin endpoints.

Add:

GET /admin/mm/permissions

Return sanitized list:

{
  "enabled": true,
  "accounts": [
    {
      "mm_account": "0x...",
      "enabled": true,
      "label": "MM Alpha",
      "can_submit_perp_orders": true,
      "can_quote_perp_rfq": true,
      "can_quote_option_rfq": true,
      "can_submit_option_orders": false
    }
  ]
}

No write endpoint in V1A.

Manual Seeding

No admin writes yet.

Document manual SQL seed examples:

INSERT INTO mm_accounts (
    mm_account,
    enabled,
    label,
    can_submit_perp_orders,
    can_quote_perp_rfq,
    can_quote_option_rfq,
    can_submit_option_orders,
    created_at_ms,
    updated_at_ms
) VALUES (...);
Tests

Normal cargo test must remain offline.

Add tests for:

permissions disabled preserves existing behavior
permissions enabled rejects missing MM account
permissions enabled rejects disabled MM account
can_quote_perp_rfq required for perp RFQ quote
can_quote_option_rfq required for option RFQ quote
can_submit_perp_orders required for submit_order if applicable
account matching still enforced with auth
market-level permission allows correct market
market-level permission rejects wrong market
option-series permission allows correct series
option-series permission rejects wrong series
admin permissions endpoint redacts and returns accounts
no admin write endpoints added
existing MM/RFQ/options tests still pass
Runtime Verification After Implementation

Use process env:

MM_GATEWAY_AUTH_MODE=wallet_challenge
MM_GATEWAY_REQUIRE_AUTH=true
MM_PERMISSIONS_ENABLED=true

Seed one MM account manually in Postgres.

Verify:

authenticated allowed MM can send option_rfq_quote
authenticated but not permissioned MM is rejected
wrong capability is rejected
admin endpoint shows permissions
Documentation

Update:

README.md
ARCHITECTURE.md
.env.example

Document:

why auth != permission
MM onboarding flow
manual approval model
config
SQL seed example
permission checks
future admin write UI
Validation

Run:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
Acceptance Criteria

Complete only if:

MM permissions config exists
DB migration exists
permission service/store exists
protected MM actions enforce capabilities when enabled
disabled mode preserves current behavior
admin read-only permissions endpoint exists
tests cover missing/disabled/wrong capability/wrong market-series
docs updated
normal tests offline
cargo fmt passes
cargo clippy passes
cargo test passes
cargo build passes
Deferred
admin write endpoints
frontend permissions UI
automatic MM onboarding
ranking/scoring
incentives/rebates
on-chain allowlist
EOF



after all, 
## run telegram.py (say nothing, just run and that all)