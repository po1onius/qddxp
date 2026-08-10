DROP TABLE IF EXISTS payment_events;

ALTER TABLE orders ADD COLUMN epay_trade_no TEXT;

UPDATE orders AS target
SET epay_trade_no = source.merchant_trade_no
FROM payment_attempts AS source
WHERE source.order_id = target.id
  AND source.provider = 'epay'
  AND source.created_at = (
      SELECT MIN(candidate.created_at)
      FROM payment_attempts AS candidate
      WHERE candidate.order_id = target.id
        AND candidate.provider = 'epay'
  );

UPDATE orders
SET epay_trade_no = replace(id::TEXT, '-', '')
WHERE epay_trade_no IS NULL;

ALTER TABLE orders ALTER COLUMN epay_trade_no SET NOT NULL;
ALTER TABLE orders ADD CONSTRAINT orders_epay_trade_no_key UNIQUE (epay_trade_no);

DROP TABLE IF EXISTS payment_attempts;

ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check
CHECK (status IN ('pending', 'paid', 'preorder'));
ALTER TABLE orders DROP CONSTRAINT orders_currency_check;
ALTER TABLE orders DROP CONSTRAINT orders_amount_cents_check;
ALTER TABLE orders DROP COLUMN expires_at;
ALTER TABLE orders DROP COLUMN currency;
ALTER TABLE orders DROP COLUMN amount_cents;
ALTER TABLE orders DROP COLUMN product_name_snapshot;
