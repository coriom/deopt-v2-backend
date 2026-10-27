-- Synthetic only. Apply exclusively to a disposable deopt_*_test database
-- after migrations and the indexer checkpoint test have completed.
INSERT INTO orders (order_id, market_id, account, side, order_type,
  time_in_force, price_1e8, size_1e8, remaining_size_1e8,
  reduce_only, post_only, client_order_id, nonce, deadline_ms, signature,
  status, created_at_ms, updated_at_ms)
VALUES ('restore-order-1', 1, 'synthetic-account-a', 'buy', 'limit', 'gtc',
  '200000000000', '100000000', '50000000', false, false,
  'restore-client-1', 1, 9999999999999, 'synthetic-no-signature',
  'partially_filled', 1000, 1001);

INSERT INTO perp_orders (id, account, market_id, side, order_type,
  price_1e8, size_1e8, remaining_size_1e8, filled_size_1e8,
  time_in_force, post_only, reduce_only, isolated_margin_1e8,
  status, client_order_id, created_at_ms, updated_at_ms)
VALUES
 ('10000000-0000-4000-8000-000000000001', 'synthetic-account-a', '1',
  'buy', 'limit', '200000000000', '100000000', '50000000', '50000000',
  'gtc', false, false, '500000000', 'partially_filled', 'restore-a', 1000, 1001),
 ('10000000-0000-4000-8000-000000000002', 'synthetic-account-b', '1',
  'sell', 'limit', '200000000000', '50000000', '0', '50000000',
  'gtc', false, false, '500000000', 'filled', 'restore-b', 1000, 1001);

INSERT INTO perp_fills (id, market_id, taker_order_id, maker_order_id,
  taker_account, maker_account, taker_side, price_1e8, size_1e8, created_at_ms)
VALUES ('20000000-0000-4000-8000-000000000001', '1',
  '10000000-0000-4000-8000-000000000002',
  '10000000-0000-4000-8000-000000000001',
  'synthetic-account-b', 'synthetic-account-a', 'sell', '200000000000',
  '50000000', 1001);

INSERT INTO perp_positions (id, account, market_id, side, size_1e8,
  entry_price_1e8, margin_1e8, realized_pnl_1e8, status,
  opened_at_ms, updated_at_ms)
VALUES ('30000000-0000-4000-8000-000000000001', 'synthetic-account-a',
  '1', 'long', '50000000', '200000000000', '500000000', '0', 'open', 1001, 1001);
