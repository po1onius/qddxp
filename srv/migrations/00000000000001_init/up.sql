-- UUID 主键统一由 PostgreSQL 生成；项目明确禁止外键，因此实体之间只保存逻辑关联 ID，
-- 并通过索引保障查询性能。业务层负责在事务中校验关联关系与维护一致性。
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- 商品定义保存面向顾客展示的稳定信息；实际可交付的卡密库存存放在 products 表。
CREATE TABLE product_info (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    image_base64 TEXT,
    name TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '',
    price_cents BIGINT NOT NULL CHECK (price_cents >= 0),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- product_info_id 不使用外键，删除商品定义等管理操作必须由应用层检查库存和订单引用。
CREATE TABLE products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_info_id UUID NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'available'
        CHECK (status IN ('available', 'reserved', 'delivered', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX products_info_status_idx ON products(product_info_id, status);
CREATE INDEX products_created_at_id_idx ON products(created_at DESC, id DESC);

-- 订单保存下单时的商品名称、金额和币种快照，后续修改商品定义不会影响已创建订单。
-- 待支付和已支付订单必须由应用层保证持有有效的 product_id；超时释放库存后清空该字段。
-- product_id 和 product_info_id 均为逻辑关联，不创建数据库外键。
CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_id UUID,
    product_info_id UUID NOT NULL,
    product_name_snapshot TEXT NOT NULL,
    amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    paid_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'paid', 'expired', 'cancelled')),
    contact TEXT NOT NULL,
    order_password_hash TEXT NOT NULL
);

CREATE INDEX orders_created_at_id_idx ON orders(created_at DESC, id DESC);
CREATE INDEX orders_contact_created_at_id_idx ON orders(contact, created_at DESC, id DESC);
CREATE INDEX orders_status_idx ON orders(status);
CREATE INDEX orders_product_info_status_idx ON orders(product_info_id, status);

-- 每次支付尝试独立保存协议、金额快照和上游状态。provider/channel 使用组合约束，
-- 从数据库层阻止 epay/native、wechatpay/wxpay 等无效组合进入系统。
CREATE TABLE payment_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    order_id UUID NOT NULL,
    provider TEXT NOT NULL,
    channel TEXT NOT NULL,
    merchant_trade_no TEXT NOT NULL UNIQUE,
    provider_transaction_id TEXT,
    state TEXT NOT NULL DEFAULT 'created'
        CHECK (state IN ('created', 'ready', 'succeeded', 'failed', 'closed')),
    code_url TEXT,
    amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ,
    CONSTRAINT payment_attempts_method_check CHECK (
        (provider = 'epay' AND channel IN ('alipay', 'wxpay'))
        OR (provider = 'wechatpay' AND channel = 'native')
    ),
    -- 平台交易号只在同一支付方的命名空间内保证唯一；不同支付方可能生成相同字符串。
    CONSTRAINT payment_attempts_provider_transaction_unique
        UNIQUE (provider, provider_transaction_id),
    -- 新系统不存在缺少平台交易号的历史成功记录；成功状态必须具备完整支付事实。
    CONSTRAINT payment_attempts_success_details_check CHECK (
        state <> 'succeeded'
        OR (provider_transaction_id IS NOT NULL AND paid_at IS NOT NULL)
    )
);

CREATE INDEX payment_attempts_order_created_at_idx
ON payment_attempts(order_id, created_at DESC);
CREATE INDEX payment_attempts_provider_state_expires_idx
ON payment_attempts(provider, state, expires_at);

-- 支付事件用于回调、主动查单等入口的幂等和审计；同一提供方事件只能入账一次。
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

-- 保存支付回调等对外 API 的结构化调用日志，便于按接口和时间排查异常。
CREATE TABLE api_call_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    api_name TEXT NOT NULL,
    http_method TEXT NOT NULL,
    path TEXT NOT NULL,
    request_params JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_status INTEGER NOT NULL,
    response_body TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX api_call_logs_api_name_created_at_idx
ON api_call_logs(api_name, created_at DESC);
CREATE INDEX api_call_logs_created_at_id_idx
ON api_call_logs(created_at DESC, id DESC);
