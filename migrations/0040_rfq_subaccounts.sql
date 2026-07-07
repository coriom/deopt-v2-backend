-- SUBACCOUNTS-RFQ-INTEGRATION-V1 — schema foundation for routing RFQ
-- state through (owner_address, subaccount_id).
--
-- Additive only. Every existing INSERT stays correct because the new
-- columns carry `NOT NULL DEFAULT 1`, so pre-migration rows land on
-- the default subaccount without any code change. Every existing
-- SELECT stays correct because repository queries enumerate explicit
-- columns.
--
-- What this migration lands:
--
-- * `option_rfqs.taker_subaccount_id` — the subaccount the taker
--   quoted from when the RFQ was created.
-- * `option_rfq_quotes.maker_subaccount_id` — the subaccount the
--   maker quoted from.
-- * `option_rfq_fills.taker_subaccount_id` +
--   `option_rfq_fills.maker_subaccount_id` — both sides captured on
--   the trade record so per-subaccount fills feeds can filter side-
--   aware, mirroring `option_fills.buyer_subaccount_id` /
--   `option_fills.seller_subaccount_id` from milestone
--   `SUBACCOUNTS-OPTIONS-ORDERS-HISTORY-V1`.
-- * Composite `(LOWER(address), subaccount_id)` indexes so future
--   subaccount-scoped queries stay cheap.
--
-- What this migration does NOT land (deferred to routing +
-- integration milestone code):
--
-- * `OptionRfqRequest` / `OptionRfqQuote` / `OptionRfqFill` structs
--   getting `taker_subaccount_id` / `maker_subaccount_id` fields.
-- * v2 canonical builders + real handler routing (added in the same
--   commit as this migration, but conceptually a separate concern).
-- * RFQ lifecycle WS event payloads carrying subaccount ids (also
--   added in the same commit).
--
-- Off-chain only. Nothing here touches Solidity, mainnet, or the
-- Perps fail-closed posture. RFQ Account 1 compatibility preserved.

-- ---------------------------------------------------------------------
-- option_rfqs — one row per taker RFQ request. `taker_subaccount_id`
-- pins the subaccount the taker signed for.
-- ---------------------------------------------------------------------

ALTER TABLE option_rfqs
    ADD COLUMN IF NOT EXISTS taker_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_rfqs
    ADD CONSTRAINT option_rfqs_taker_subaccount_id_positive
    CHECK (taker_subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_rfqs_taker_subaccount
    ON option_rfqs (LOWER(taker), taker_subaccount_id);

-- ---------------------------------------------------------------------
-- option_rfq_quotes — one row per maker quote. `maker_subaccount_id`
-- pins the subaccount the maker signed for. The maker's on-wire
-- account field is `mm_account`; the DB column name uses `maker_`
-- prefix to stay symmetric with the taker column above.
-- ---------------------------------------------------------------------

ALTER TABLE option_rfq_quotes
    ADD COLUMN IF NOT EXISTS maker_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_rfq_quotes
    ADD CONSTRAINT option_rfq_quotes_maker_subaccount_id_positive
    CHECK (maker_subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_rfq_quotes_maker_subaccount
    ON option_rfq_quotes (LOWER(mm_account), maker_subaccount_id);

-- ---------------------------------------------------------------------
-- option_rfq_fills — event log. Two subaccount columns, one per party.
-- Both DEFAULT 1 so the current accept path (which writes fills
-- directly) continues to write correct rows. Handler code copies
-- these from the RFQ + quote rows on accept.
-- ---------------------------------------------------------------------

ALTER TABLE option_rfq_fills
    ADD COLUMN IF NOT EXISTS taker_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_rfq_fills
    ADD COLUMN IF NOT EXISTS maker_subaccount_id INTEGER NOT NULL DEFAULT 1;

ALTER TABLE option_rfq_fills
    ADD CONSTRAINT option_rfq_fills_taker_subaccount_id_positive
    CHECK (taker_subaccount_id >= 1);

ALTER TABLE option_rfq_fills
    ADD CONSTRAINT option_rfq_fills_maker_subaccount_id_positive
    CHECK (maker_subaccount_id >= 1);

CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_taker_subaccount
    ON option_rfq_fills (LOWER(taker), taker_subaccount_id);

CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_maker_subaccount
    ON option_rfq_fills (LOWER(mm_account), maker_subaccount_id);
