-- HISTORY-V2-TERMINAL-REASONS-V1
--
-- Persist real terminal reasons on option orders so `/history` can
-- distinguish user cancel from IOC remainder cancel without inferring
-- from `status + time_in_force + post_only`.
--
-- Three nullable fields are added. Pre-persistence rejections
-- (PostOnlyWouldMatch, FokNotFillable, matching rejections) cannot be
-- recorded here because no order row is ever inserted — they remain
-- synchronous errors at submit time. See
-- `docs/HISTORY_V2_TERMINAL_REASONS_V1_RESULT.md` for the full list
-- of populable vs. pre-persistence cases.
--
-- Field shape (mirrors the brief's preferred `terminal_reason_*`
-- triple; the conditional_orders convention of `failure_code` /
-- `failure_message` does not fit the `user_cancelled` semantics
-- since a user cancel is not a failure).
--
--   terminal_reason_code     -- snake_case enum-like token (e.g. 'user_cancelled')
--   terminal_reason_message  -- optional user-facing detail (NULL for now)
--   terminal_reason_source   -- where the transition was authored
--                                ('user' | 'tif_policy' | 'system' | …)
--
-- No backfill: existing rows keep NULL reason fields and the
-- frontend's TIF-derived fallback continues to render them. Only
-- new transitions stamp the persisted reason.
--
-- No new indexes: the column is read alongside the row in existing
-- per-account / per-order queries; we don't filter by it.

ALTER TABLE option_orders
    ADD COLUMN IF NOT EXISTS terminal_reason_code TEXT NULL,
    ADD COLUMN IF NOT EXISTS terminal_reason_message TEXT NULL,
    ADD COLUMN IF NOT EXISTS terminal_reason_source TEXT NULL;
