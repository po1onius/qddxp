-- 将订单本身与具体支付协议解耦。项目要求禁止外键，因此以下关联列仅创建索引。
ALTER TABLE orders ADD COLUMN product_name_snapshot TEXT;
ALTER TABLE orders ADD COLUMN amount_cents BIGINT;
ALTER TABLE orders ADD COLUMN currency TEXT NOT NULL DEFAULT 'CNY';
ALTER TABLE orders ADD COLUMN expires_at TIMESTAMPTZ;

UPDATE orders AS target
SET product_name_snapshot = source.name,
    amount_cents = source.price_cents,
    expires_at = target.created_at + INTERVAL '15 minutes'
FROM product_info AS source
WHERE source.id = target.product_info_id;

ALTER TABLE orders ALTER COLUMN product_name_snapshot SET NOT NULL;
ALTER TABLE orders ALTER COLUMN amount_cents SET NOT NULL;
ALTER TABLE orders ALTER COLUMN expires_at SET NOT NULL;
ALTER TABLE orders ADD CONSTRAINT orders_amount_cents_check CHECK (amount_cents >= 0);
ALTER TABLE orders ADD CONSTRAINT orders_currency_check CHECK (currency = 'CNY');
ALTER TABLE orders DROP CONSTRAINT orders_status_check;
ALTER TABLE orders ADD CONSTRAINT orders_status_check
CHECK (status IN ('pending', 'paid', 'preorder', 'expired', 'cancelled'));

CREATE TABLE payment_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id UUID NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('epay', 'wechatpay')),
    channel TEXT NOT NULL CHECK (channel IN ('alipay', 'wxpay', 'native', 'legacy')),
    merchant_trade_no TEXT NOT NULL UNIQUE,
    provider_transaction_id TEXT UNIQUE,
    state TEXT NOT NULL DEFAULT 'created'
        CHECK (state IN ('created', 'prepay_created', 'succeeded', 'failed', 'closed')),
    code_url TEXT,
    amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ
);

CREATE INDEX payment_attempts_order_created_at_idx
ON payment_attempts(order_id, created_at DESC);
CREATE INDEX payment_attempts_provider_state_expires_idx
ON payment_attempts(provider, state, expires_at);

-- 老数据来自原有 ePay 实现，但历史订单没有保存 alipay/wxpay 选择，统一标记为 legacy。
INSERT INTO payment_attempts (
    id,
    order_id,
    provider,
    channel,
    merchant_trade_no,
    state,
    amount_cents,
    currency,
    expires_at,
    created_at,
    updated_at,
    paid_at
)
SELECT
    gen_random_uuid(),
    id,
    'epay',
    'legacy',
    epay_trade_no,
    CASE WHEN status IN ('paid', 'preorder') THEN 'succeeded' ELSE 'created' END,
    amount_cents,
    currency,
    expires_at,
    created_at,
    COALESCE(paid_at, created_at),
    paid_at
FROM orders;

ALTER TABLE orders DROP COLUMN epay_trade_no;

CREATE TABLE payment_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider TEXT NOT NULL CHECK (provider IN ('epay', 'wechatpay')),
    provider_event_id TEXT NOT NULL,
    payment_attempt_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    request_body TEXT NOT NULL DEFAULT '',
    success BOOLEAN NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (provider, provider_event_id)
);

CREATE INDEX payment_events_attempt_created_at_idx
ON payment_events(payment_attempt_id, created_at DESC);
