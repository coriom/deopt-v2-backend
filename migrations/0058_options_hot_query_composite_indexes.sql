-- OPTIONS-HYBRID-V2-BACKEND-FINAL-CLOSURE-V1 Part H:
-- Additive composite / sparse indexes for Options hot query paths.
-- Every index is IF NOT EXISTS and does not modify any existing
-- schema, so this migration is safe to apply to a live database.
--
-- See docs/OPTIONS_HYBRID_V2_BACKEND_FINAL_CLOSURE_V1.md for the
-- audit that identified these indexes.

-- 1. option_fills: series + buyer per-account history read.
--    Backs `list_option_fills_for_account_and_series` which
--    currently OR's buyer/seller against separate single-column
--    indexes.
CREATE INDEX IF NOT EXISTS idx_option_fills_series_buyer_created
    ON option_fills (option_series_id, LOWER(buyer), created_at_ms);

-- 2. option_fills: series + seller per-account history read.
CREATE INDEX IF NOT EXISTS idx_option_fills_series_seller_created
    ON option_fills (option_series_id, LOWER(seller), created_at_ms);

-- 3. option_orders: series + side + live status for the matcher's
--    FOR UPDATE critical section.
CREATE INDEX IF NOT EXISTS idx_option_orders_series_side_live
    ON option_orders (option_series_id, side)
    WHERE status IN ('open', 'partially_filled');

-- 4. option_orders: live status + deadline for the expiry sweep.
CREATE INDEX IF NOT EXISTS idx_option_orders_status_deadline
    ON option_orders (deadline_ms)
    WHERE status IN ('open', 'partially_filled') AND deadline_ms IS NOT NULL;

-- 5. option_twap_orders: scheduler due-list scan.
CREATE INDEX IF NOT EXISTS idx_option_twap_orders_status_execution
    ON option_twap_orders (next_execution_at_ms)
    WHERE status IN ('pending', 'running');

-- 6. option_reservations: execution-id + purpose for the reorg
--    reactivation path (existing sparse index covers execution_id
--    only; this narrows to PENDING_SETTLEMENT).
CREATE INDEX IF NOT EXISTS idx_option_reservations_pending_by_execution
    ON option_reservations (canonical_execution_id, status)
    WHERE canonical_execution_id IS NOT NULL AND purpose = 'PENDING_SETTLEMENT';

-- 7. options_conditional_orders: OCO cancel scan (existing index
--    covers oco_group_id only; this narrows to armed status).
CREATE INDEX IF NOT EXISTS idx_options_conditional_orders_oco_armed
    ON options_conditional_orders (oco_group_id)
    WHERE oco_group_id IS NOT NULL AND status = 'armed';

-- 8. option_orders: account + subaccount + series + status composite.
--    Backs future scoped-read routes (list_orders_for_owner_sub).
CREATE INDEX IF NOT EXISTS idx_option_orders_account_subaccount_series_status
    ON option_orders (LOWER(account), subaccount_id, option_series_id, status);

-- 9. option_execution_intents: source + status filter for
--    admin/lifecycle inspection.
CREATE INDEX IF NOT EXISTS idx_option_execution_intents_source_status
    ON option_execution_intents (source_type, status);

-- 10. option_execution_correlations: status + updated_at for the
--     reconciliation worker's stale-correlation scan.
CREATE INDEX IF NOT EXISTS idx_option_execution_correlations_status_updated
    ON option_execution_correlations (correlation_status, last_updated_at_ms);

-- 11. option_rfq_fills: taker + subaccount per-account RFQ history.
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_taker_subaccount_created
    ON option_rfq_fills (LOWER(taker), taker_subaccount_id, created_at_ms);

-- 12. option_rfq_fills: maker + subaccount per-account RFQ history.
CREATE INDEX IF NOT EXISTS idx_option_rfq_fills_maker_subaccount_created
    ON option_rfq_fills (LOWER(mm_account), maker_subaccount_id, created_at_ms);
