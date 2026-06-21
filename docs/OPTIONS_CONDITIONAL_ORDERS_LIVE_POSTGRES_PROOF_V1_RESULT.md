# OPTIONS-CONDITIONAL-ORDERS-LIVE-POSTGRES-PROOF-V1 — Result

Live-PostgreSQL proof attempt for the existing TP/SL system.
Updated under the follow-up milestone
`OPTIONS-CONDITIONAL-ORDERS-PG-UNLOCK-AND-CLOSE-V1` to add the
**PRECREATED_DB mode** that lets the operator unblock the live run
without granting CREATEDB to the normal application role and
without re-exposing any secret to the agent.

## Status

**BLOCKED — live PostgreSQL run not executed.**

Follow-up attempt under
`OPTIONS-CONDITIONAL-ORDERS-POSTGRES-FINAL-CLOSURE-V1` reached
Phase 2 (passwordless admin DB-creation probe) and **stopped
without changing anything**, exactly as the brief prescribes:

```
$ sudo -n -u postgres true
sudo: a password is required
true_exit=1
```

The agent cannot supply the local `sudo` password
non-interactively, and the application role
(`DATABASE_URL`) holds no `CREATEDB` privilege — and per the
safety brief MUST NOT be granted any. The runner script itself
is fully ready: both PRECREATED_DB and SELF_MANAGED_DB modes
shipped + gated + name-allow-listed; only the actual DB
creation step requires the operator.

**Counts** (from the final validation sweep of this conversation):
* PG-proof tests **executed**: **0**
* PG-proof tests **passed**: **0**
* PG-proof tests **skipped**: **8**

(8 skipped because `CONDITIONAL_PG_TEST_DATABASE_URL` is unset.
Per brief: skip mode does NOT count as proof. The TP/SL
subsystem is therefore **NOT fully closed**.)

## Runner architecture (after this milestone)

`scripts/conditional-orders-pg-proof.sh` now supports two safe
modes:

### PRECREATED_DB mode (preferred)

```text
export CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true
export CONDITIONAL_PG_TEST_DATABASE_URL=...     # operator sets
./scripts/conditional-orders-pg-proof.sh
```

* Never sources `deopt-v2-backend/.env`.
* Never calls `CREATE DATABASE` or `DROP DATABASE`.
* Never prints `CONDITIONAL_PG_TEST_DATABASE_URL` (only the
  database name extracted via a stdin-piped python parser,
  matched against an allow-list).
* Single safe stdout line:
  `Using operator-provided disposable PostgreSQL database (name
  passes allow-list).`
* Cleanup of the disposable database is the operator's
  responsibility in this mode.

### SELF_MANAGED_DB mode (fallback)

```text
export CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true
./scripts/conditional-orders-pg-proof.sh
```

* Sources `.env` to derive a server prefix (silently, no `bash -x`).
* Creates `deopt_cond_proof_<utc-ts>_<8-hex>` with `CREATE
  DATABASE`, drops it in a trap on exit.
* Refuses with `BLOCKED` exit code 2 if the role lacks `CREATEDB`.
* The script does NOT grant `CREATEDB` to anything; it merely
  emits an actionable message pointing the operator at the
  PRECREATED_DB mode or at creating a dedicated `deopt_test`
  role.

### Common safety gates (both modes)

* **Operator confirmation required**:
  `CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true` must be set. The
  script refuses with exit=2 otherwise.
* **Database-name allow-list**: only names starting with
  `deopt_cond_proof_` or `deopt_test_` are accepted. Anything
  else (including `deopt_v2_backend`, `prod_*`, `mainnet_*`, any
  operator/dev name) is refused. Safety is NEVER inferred from
  `localhost`.
* **Mainnet block**: refuses if `CHAIN_ID` resolves to `1` or
  `8453`.
* **Secret wiping pre-cargo**: before invoking `cargo test`, the
  script unsets `EXECUTOR_PRIVATE_KEY`, `BUYER_PRIVATE_KEY`,
  `SELLER_PRIVATE_KEY`, `RPC_URL`, `ADMIN_BEARER_TOKEN` so any
  test regression cannot pick them up by accident.
* **DATABASE_URL wipe**: in SELF_MANAGED_DB mode, the operator
  `DATABASE_URL` is `unset` before cargo runs so a misbehaving
  test cannot accidentally target the operator database.
* **No `bash -x` warning** baked into the script header: the
  previous incident is referenced inline as a comment.

## Pre-created database setup — operator instructions

The operator runs ONE of the two paths as a PostgreSQL
**superuser** (not the application role).

### Path A — PRECREATED_DB mode (recommended)

```sql
-- as postgres superuser
CREATE DATABASE deopt_cond_proof_manual_<tag>
  OWNER <existing_app_role>;   -- or a separate deopt_test role
```

Then in the operator's shell (NOT in `.env`, NOT committed):

```text
export CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true
export CONDITIONAL_PG_TEST_DATABASE_URL=...
./scripts/conditional-orders-pg-proof.sh
```

When done, the operator runs:

```sql
DROP DATABASE deopt_cond_proof_manual_<tag>;
```

### Path B — SELF_MANAGED_DB mode (also acceptable)

```sql
-- as postgres superuser; NEVER grant CREATEDB to the production
-- application role.
CREATE ROLE deopt_test LOGIN PASSWORD '<set>' CREATEDB;
```

Provide a connection URL for `deopt_test` (via env var, not
`.env`) and run the script without `CONDITIONAL_PG_TEST_DATABASE_URL`
set — the script will create + drop its own disposable database.

## Migration result

Deferred. The migration path in the test crate is
`PgRepository::run_migrations()` — exactly the same code the
backend runs at startup (`src/main.rs:60-62`). The first PG-proof
test asserts via raw SQL that the post-migration schema includes
the table, the four indexes, the `version` column, and the
`option_orders` UNIQUE INDEX on `(account, client_order_id)`
that prevents duplicate child orders.

## Repository-path confirmation

Architecturally confirmed (the test harness asserts both invariants
**before** any test logic runs):
```rust
let state = AppState::with_options_config_and_repository(...);
assert!(state.repository.is_some(),
        "repository must be wired for the PG proof");
assert!(state.persistence_enabled,
        "persistence_enabled must be true so DB code paths are used");
```
The `AppState::with_options_config_and_repository` builder
explicitly sets `state.repository = Some(repo)` and
`state.persistence_enabled = true`. There is no in-memory
fallback in the PG-proof test crate.

## SQL / repository assertions (deferred until execution)

When the operator unblocks via PRECREATED_DB or SELF_MANAGED_DB
mode, the suite asserts (per scenario, via parameterised
`sqlx::query_scalar` against the disposable DB):

| Query | Expected (race scenario) |
|---|---|
| `COUNT(*) FROM options_conditional_orders WHERE option_series_id=$1` | **2** |
| `... WHERE status='completed'` | **1** |
| `... WHERE status='cancelled' AND failure_code='oco_sibling_triggered'` | **1** |
| `COUNT(*) FROM option_orders WHERE client_order_id LIKE 'cond-%'` | **1** |
| Same query after 5 more ticks | **1** (unchanged) |
| `COUNT(*) FROM option_fills ... linked to child` | **≥ 1** |
| `child.time_in_force` | **`ioc`** (per the column read by `option_order_from_row`) |
| `child.post_only` | **`false`** |
| `child.status` after IOC matching | **`filled` or `cancelled`** (NEVER `open` / `partially_filled`) |
| Position signed-fills invariant for HOLDER | **≥ 0** (no reversal) |

The reduce-only column `reduce_only=true` is enforced server-side
in `execute_triggered_via_repo` (the service builds the child
with `reduce_only: true` and the database table default also
ensures it).

## Concurrency proof architecture

The PG-proof suite's race test
(`two_workers_compete_for_oco_group_one_winner_only`) uses:

* **Independent connections**: each tokio task receives an
  `Arc<AppState>` and the underlying `sqlx::PgPool` distributes
  connections from its pool — concurrent ticks acquire SEPARATE
  pooled connections from the same pool.
* **NOT two OS processes**: this conversation does not launch two
  `cargo run` processes. The honest description of the proof is
  "concurrent connections", not "cross-process".

A separate optional script (deferred until operator unblock) is
NOT shipped in this milestone because the brief explicitly allows
the concurrent-connections proof: *"The repository-backed
concurrent test is sufficient for closure if it uses independent
DB connections and validates atomic SQL state."*

The atomic cross-process safety guarantee comes from the
`UPDATE options_conditional_orders SET status='triggering' WHERE
id = $1 AND status = 'armed' RETURNING ...` primitive in
`Repository::claim_conditional_order_armed` — the same primitive
two separate OS processes would contend on against the same
database.

## Bugs found and fixes applied

In this follow-up milestone:

| # | Bug | Fix |
|---|---|---|
| 1 | `extract_dbname` python heredoc consumed stdin from the heredoc itself instead of the piped URL — name guard would always fail with "URL has no database path". | Replaced `python3 - <<'PY' … PY` with `python3 -c '…'` so the heredoc no longer interferes with stdin. Same fix applied to the SELF_MANAGED-mode URL substitution. |
| 2 | Single-mode script could not be unblocked without granting CREATEDB. | Added PRECREATED_DB mode which never creates/drops, never sources `.env`, never echoes the URL. |
| 3 | Both modes could run without operator confirmation. | Added `CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true` opt-in gate. |
| 4 | Database-name guard previously checked only `prod_*`/`mainnet_*` prefixes (allow-by-default). | Switched to allow-LIST (`deopt_cond_proof_*` and `deopt_test_*` only). |
| 5 | Inherited secrets could leak into the cargo test environment if a future test regression read them. | Pre-cargo unset of `EXECUTOR_PRIVATE_KEY`, `BUYER_PRIVATE_KEY`, `SELLER_PRIVATE_KEY`, `RPC_URL`, `ADMIN_BEARER_TOKEN`. |

No bug in the conditional-orders system itself.

## Secret-rotation checklist (variable names only — never values)

Per the incident in the previous milestone, the following
operator-controlled secrets were echoed to a temporary log file
by `bash -x`, captured by the agent transcript context, then
shredded (4-pass random + zero + unlink). Even though the values
are testnet-only, hygiene requires rotation.

The operator must replace ALL of the following — the agent does
NOT see or request the new values:

| Variable name in `deopt-v2-backend/.env` | Category | Action |
|---|---|---|
| `EXECUTOR_PRIVATE_KEY` | Executor testnet wallet private key | Generate a new keypair; replace the value in-file; update any operator-side dashboards. |
| `BUYER_PRIVATE_KEY` | Buyer testnet wallet private key | Same as above. |
| `SELLER_PRIVATE_KEY` | Seller testnet wallet private key | Same as above. |
| `RPC_URL` | RPC provider API credential | Issue a new API key from the provider; update env. |
| `DATABASE_URL` | PostgreSQL password | Change the role password (`ALTER ROLE … WITH PASSWORD '<new>'`); update env. |

The agent never sees the replacements. Do NOT paste new values
into chat.

**Confirmation that no secret value was printed in this
milestone**: no `bash -x` was used; both probe runs in this
conversation captured only `[pg-proof] BLOCKED …` and the
extracted database NAME (`deopt_v2_backend`, used only because
the operator-supplied synthetic URL contained it as the dbname
segment — the URL itself was never echoed). All temporary log
files were shredded after each probe.

## Test-runner isolation from unrelated secrets

`tests/conditional_orders_pg_proof.rs` reads ONLY one env var:
`CONDITIONAL_PG_TEST_DATABASE_URL`. Verified via:
```text
grep -n 'std::env::var' tests/conditional_orders_pg_proof.rs
69:    std::env::var(ENV_VAR).ok().filter(|v| !v.is_empty())
694:   std::env::var(ENV_VAR).expect("ENV var must be set …");
```
No wallet private key, no RPC URL, no signer, no admin bearer.
The proof can run with NONE of those configured. The runner
script additionally wipes the canonical secret env vars before
invoking `cargo test` as defence-in-depth.

## Files changed in this follow-up milestone

| File | Change |
|---|---|
| `scripts/conditional-orders-pg-proof.sh` | Rewritten. PRECREATED_DB + SELF_MANAGED_DB modes, `CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true` gate, allow-list database name guard, python `-c` invocation (fixes heredoc/stdin collision), pre-cargo unset of canonical secret env vars, header comment warning against `bash -x`. |
| `docs/OPTIONS_CONDITIONAL_ORDERS_LIVE_POSTGRES_PROOF_V1_RESULT.md` | This document, updated. |
| `RUN_STATE.md` | New `PG-UNLOCK-AND-CLOSE-V1` milestone block prepended. |

**No production code path was modified.** No change to
`src/options/conditional_orders.rs`, no change to
`src/db/repository.rs`, no change to `src/api/http.rs`, no
change to `src/main.rs`, no change to migrations, no change to
the PG-proof test file's logic.

## Validation results

* `cargo fmt --check` clean.
* `cargo check --lib` clean.
* `cargo check --bin deopt-v2-backend` clean.
* `cargo test --test conditional_orders_pg_proof` → **8 tests
  skipped (env unset)** — NOT counted as proof.
* `cargo test --test conditional_orders_tests` → **12/12 ok**
  (V1 milestone — unchanged).
* `cargo test --test conditional_orders_e2e_tests` → **12/12 ok**
  (V1-E2E milestone — unchanged).
* `cargo test --test options_tests` → **88/88 ok**.
* `cargo test --lib` → **1013/1013 ok**.
* `git diff --check` clean.

Script gate probes (no real secret; synthetic URL):
* Without confirmation var → exit=2, clean BLOCKED message, zero
  URL output.
* With confirmation var + URL whose dbname is `deopt_v2_backend`
  → exit=2, name-allow-list refusal, zero URL output, NAME
  printed (the segment between `/` and `?` of the synthetic URL).

## Safety posture

* **No mainnet.** No chain transaction sent. No signer loaded.
  Broadcast disabled throughout.
* **No deployment.** No Solidity touched.
* **No production database mutated.** No `CREATE`/`DROP`
  performed in this milestone (the script's create path requires
  CREATEDB which the role does not have; the BLOCKED-state run
  exits before any DB-level write).
* **No DATABASE_URL printed.** No RPC URL. No private key. No
  password. No signature.
* **No `bash -x`** on any script that sources `.env` (script
  header warns against this; agent rule reinforced).
* **No operator approval consumed** because no operation
  requiring one was attempted.
* **No chain transaction occurred.**

## Whether the TP/SL subsystem can now be declared fully closed

**NO.** The closure criteria from the brief are explicit:
* PostgreSQL proof executed for real ← NOT YET
* all 8 tests passed ← 0 passed (0 executed)
* zero tests skipped ← 8 skipped
* migration `0028` actually applied ← NOT YET
* OCO race produced one winner and one child ← NOT YET
* no secret was exposed during this run ← TRUE for THIS milestone

Five of six closure criteria require the live execution. The
subsystem CANNOT be declared closed until the operator runs the
script in PRECREATED_DB mode against a real disposable database
and the 8 tests report `8 executed / 8 passed / 0 skipped`.

## Next recommendation — minimal operator command sequence

The operator runs the following three commands in their own
shell. The agent does NOT see the URL or the password.

### Step 1 — As a PostgreSQL superuser (NOT the application role)

```sh
sudo -u postgres createdb \
  --owner='<existing_app_role>' \
  'deopt_cond_proof_final_<short_random_tag>'
```

### Step 2 — In the operator's interactive shell

```sh
export CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB=true
export CONDITIONAL_PG_TEST_DATABASE_URL='<connection_url_to_the_new_database>'

./scripts/conditional-orders-pg-proof.sh

# Expected: `8 executed / 8 passed / 0 skipped`.
# The script prints only:
#   [pg-proof] mode: PRECREATED_DB
#   [pg-proof] Using operator-provided disposable PostgreSQL database (name passes allow-list).
#   ... per-test progress ...
#   [pg-proof] tests completed (PRECREATED_DB mode — cleanup is the operator's responsibility)

unset CONDITIONAL_PG_TEST_DATABASE_URL CONDITIONAL_PG_TEST_ALLOW_DISPOSABLE_DB
```

### Step 3 — As superuser, drop the disposable database

```sh
sudo -u postgres dropdb 'deopt_cond_proof_final_<short_random_tag>'
```

### Step 4 — Update RUN_STATE.md

Only if the test output reports `8 executed / 8 passed / 0
skipped`, prepend a new block to `RUN_STATE.md` marking
`OPTIONS-CONDITIONAL-ORDERS-POSTGRES-FINAL-CLOSURE-V1 — CLOSED`
**and** `OPTIONS TP/SL SUBSYSTEM — FULLY CLOSED`.

### Operator acceptance — wallet rotation deferred

The operator has explicitly accepted that the previously exposed
testnet wallet keys + RPC credential + DB password remain unused
in production-equivalent contexts. **Wallet rotation is NOT a
blocker for the TP/SL closure decision** under this milestone.
