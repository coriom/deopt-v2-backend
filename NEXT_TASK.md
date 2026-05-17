# NEXT_TASK.md — CI V1A: Backend Static Validation

## Context

The backend now has:

- protocol flows
- MM auth/permissions
- option/perp fee ledger
- admin endpoints
- E2E local harness

Runtime verification passed locally.

Missing layer:

Automated CI validation on GitHub.

## Goal

Implement CI V1A for the backend repository.

Add GitHub Actions workflow that runs on:

- pull_request
- push to main

The workflow must validate:

- Rust formatting
- Rust clippy
- Rust tests
- Rust build
- E2E harness syntax/help

## Non-Goals

Do not run live Postgres E2E in CI yet.
Do not run Base Sepolia RPC flows.
Do not require private keys.
Do not require WebTransport certs.
Do not deploy.
Do not modify Solidity.
Do not push.

## Safety Rules

CI must not require secrets.

CI must not call:

- /executor/broadcast
- RPC write endpoints
- live private-key flows

CI must not use production .env.

## Workflow

Add:

```text
.github/workflows/backend-ci.yml

Suggested jobs:

backend

Steps:

checkout
install stable Rust toolchain
cache cargo if appropriate
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
python3 -m py_compile scripts/e2e/run_e2e.py
python3 scripts/e2e/run_e2e.py --help
Environment

Use safe environment only if needed:

EXECUTION_ENABLED=false
EXECUTOR_REAL_BROADCAST_ENABLED=false
MM_GATEWAY_ENABLED=false

But normal cargo tests should not need runtime env.

Optional

If repo already has SQLx offline requirements, handle them cleanly.

Do not add brittle hacks.

Documentation

Update README with:

CI status / validation commands

Add a short section:

Local validation

with:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
python3 scripts/e2e/run_e2e.py --help
Validation

Run locally:

cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build
python3 -m py_compile scripts/e2e/run_e2e.py
python3 scripts/e2e/run_e2e.py --help
Acceptance Criteria

Complete only if:

GitHub Actions workflow exists
no secrets required
backend fmt/clippy/test/build included
E2E harness py_compile/help included
README documents local validation
local validation passes
Deferred
Postgres service CI
runtime E2E CI
frontend CI
Solidity CI
deployment workflows
EOF