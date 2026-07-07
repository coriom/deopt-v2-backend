-- SUBACCOUNTS-OPTIONS-ORDERS-HISTORY-V1 — schema foundation for
-- routing Options trading state through (owner_address, subaccount_id).
--
-- This migration is **additive only**. Every existing INSERT stays
-- correct because the new columns carry `NOT NULL DEFAULT 1`, so
-- rows land on the default subaccount without any code change. Every
-- existing SELECT stays correct because it enumerates explicit
-- columns (see e.g. `insert_option_order` in `src/db/repository.rs`).
--
-- What this milestone lands:
--
-- * DB columns on the four Options tables that hold account identity.
-- * Composite `(LOWER(account), subaccount_id)` indexes so future
--   subaccount-scoped queries stay cheap.
--
-- What this milestone does NOT land (deferred to the follow-up
-- milestone `SUBACCOUNTS-OPTIONS-ROUTING-V1`):
--
-- * `OptionOrder` / `OptionFill` / conditional / TWAP Rust structs
--   getting `subaccount_id` fields.
-- * Handler routing of `subaccount_id` into the persisted row.
-- * v2 write-auth actually reaching mutation. This milestone
--   fails-closed with `SubaccountsRoutingNotLive` at handler entry
--   whenever an envelope declares `version = 2`.
-- * WS lifecycle events carrying `subaccount_id`.
--
-- Rationale: the migration is **honest and reversible**. A future
-- routing milestone can drop the DEFAULT + flip the fail-closed
-- gate without needing another schema migration.
--
-- Off-chain only. Nothing here touches Solidity, mainnet, or the
-- Perps fail-closed posture.

-- ---------------------------------------------------------------------
-- option_orders — one row per taker order that reached persistence.
-- Trading routes stay wallet-only in this milestone; DB DEFAULT 1
-- guarantees existing INSERTs still work.
-- ---------------------------------------------------------------------

ALTER TABLE option_orders
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_orders
    ADD CONSTRAINT option_orders_subaccount_id_positive
    CHECK (subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_orders_account_subaccount
    ON option_orders (LOWER(account), subaccount_id);

-- ---------------------------------------------------------------------
-- option_fills — event log. Two subaccount columns, one per party.
-- Both DEFAULT 1 so the current match engine (which does not yet
-- carry subaccount) continues to write correct rows.
-- ---------------------------------------------------------------------

ALTER TABLE option_fills
    ADD COLUMN IF NOT EXISTS buyer_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_fills
    ADD COLUMN IF NOT EXISTS seller_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_fills
    ADD CONSTRAINT option_fills_buyer_subaccount_id_positive
    CHECK (buyer_subaccount_id >= 1);

ALTER TABLE option_fills
    ADD CONSTRAINT option_fills_seller_subaccount_id_positive
    CHECK (seller_subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_fills_buyer_subaccount
    ON option_fills (LOWER(buyer), buyer_subaccount_id);

CREATE INDEX IF NOT EXISTS idx_option_fills_seller_subaccount
    ON option_fills (LOWER(seller), seller_subaccount_id);

-- ---------------------------------------------------------------------
-- options_conditional_orders — attached TP/SL persistence.
-- ---------------------------------------------------------------------

ALTER TABLE options_conditional_orders
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE options_conditional_orders
    ADD CONSTRAINT options_conditional_orders_subaccount_id_positive
    CHECK (subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_account_subaccount
    ON options_conditional_orders (LOWER(account), subaccount_id);

-- ---------------------------------------------------------------------
-- option_twap_orders — parent TWAP intent. Children inherit via the
-- FK to `option_twap_orders(option_twap_id)`; no direct column added
-- on `option_twap_child_orders` in this milestone.
-- ---------------------------------------------------------------------

ALTER TABLE option_twap_orders
    ADD COLUMN IF NOT EXISTS subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_twap_orders
    ADD CONSTRAINT option_twap_orders_subaccount_id_positive
    CHECK (subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_twap_orders_account_subaccount
    ON option_twap_orders (LOWER(account), subaccount_id);
