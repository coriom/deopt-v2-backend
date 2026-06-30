-- HISTORY-V2-REJECTED-ATTEMPTS-FEED-V1
--
-- Persist option order submit attempts that are rejected BEFORE an
-- `option_orders` row is created so /history can surface them as
-- "rejected" entries. Today the matching policy emits a synchronous
-- HTTP error (post-only-would-match, FOK-not-fillable, …) and the
-- attempt disappears: the tester has no record that they tried.
--
-- This new table records only attempts where the caller's identity
-- is safely verified at the rejection point — write-auth has
-- already passed and the caller's account matches the order's
-- `account` field. Auth-level errors (invalid signature, signer
-- mismatch, expired challenge) are never recorded as trader
-- history.
--
-- Recordable reasons (snake_case tokens, see
-- src/options/types.rs::rejection_reason for the canonical list):
--
--   post_only_would_match     (matching_policy)
--   fok_not_fillable          (matching_policy)
--   self_trade                (matching_policy)
--   deadline_expired          (request_validation)
--   zero_price                (request_validation)
--   zero_size                 (request_validation)
--   unsupported_tif           (request_validation)
--   invalid_tif_combination   (request_validation)
--   option_series_inactive    (series_state)
--
-- The submitted parameters are stored as captured (price + size as
-- TEXT to match the existing `option_orders` convention; TIF and
-- side mirror the enum's snake_case `as_str()` representation).
-- NO signature, authorization envelope, request body, RPC URL or
-- token is stored — those carry no trader-visible information.

CREATE TABLE IF NOT EXISTS option_order_rejections (
    rejection_id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    option_series_id TEXT NULL,
    side TEXT NULL,
    price_1e8 TEXT NULL,
    size_1e8 TEXT NULL,
    time_in_force TEXT NULL,
    post_only BOOLEAN NULL,
    client_order_id TEXT NULL,
    nonce TEXT NULL,
    reason_code TEXT NOT NULL,
    reason_message TEXT NULL,
    reason_source TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

-- Mirrors the option_orders account index: case-insensitive on
-- account, newest first. /history queries fetch the per-account
-- slice already filtered by `since_ms` so this single index is
-- sufficient.
CREATE INDEX IF NOT EXISTS idx_option_order_rejections_account_time
    ON option_order_rejections (lower(account), created_at_ms DESC);
