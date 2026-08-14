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
-- 卡密内容可能包含较长的多行文本，直接为 content 建 B-tree 唯一索引可能超过 PostgreSQL
-- 单条索引记录大小限制。使用 pgcrypto 的 SHA-256 摘要建立同商品内唯一索引，既能阻止
-- 同一批次、重复上传和并发上传造成的重复库存，也不会把卡密明文复制进额外索引。
CREATE UNIQUE INDEX products_info_content_sha256_unique_idx
ON products(product_info_id, digest(content, 'sha256'));

-- 订单保存下单时的商品名称、金额和币种快照，后续修改商品定义不会影响已创建订单。
-- 待支付和已交付订单持有库存 ID；超时或取消释放库存时必须清空该字段。
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
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'delivered', 'expired')),
    -- PostgreSQL 的 char_length 按字符而非 UTF-8 字节计数，与应用层校验语义一致。
    contact TEXT NOT NULL CHECK (char_length(contact) <= 50),
    order_password_hash TEXT NOT NULL,
    CONSTRAINT orders_inventory_state_check CHECK (
        (status IN ('pending', 'delivered') AND product_id IS NOT NULL)
        OR (status = 'expired' AND product_id IS NULL)
    )
);

CREATE INDEX orders_created_at_id_idx ON orders(created_at DESC, id DESC);
CREATE INDEX orders_contact_created_at_id_idx ON orders(contact, created_at DESC, id DESC);
CREATE INDEX orders_product_info_status_idx ON orders(product_info_id, status);
CREATE INDEX orders_status_expires_idx ON orders(status, expires_at);

-- 每张订单只创建一条支付尝试，独立保存协议、金额快照和上游支付事实。
-- provider/channel 使用组合约束，从数据库层阻止无效组合进入系统。
CREATE TABLE payment_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 当前业务规定一张订单只允许一个支付尝试，避免支付期限和支付事实出现多个来源。
    order_id UUID NOT NULL UNIQUE,
    provider TEXT NOT NULL,
    channel TEXT NOT NULL,
    merchant_trade_no TEXT NOT NULL UNIQUE,
    provider_transaction_id TEXT,
    state TEXT NOT NULL DEFAULT 'created'
        CHECK (state IN ('created', 'ready', 'succeeded', 'failed', 'closed')),
    code_url TEXT,
    amount_cents BIGINT NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
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

-- 微信支付超时任务按 updated_at 轮转失败候选，避免少量持续失败的旧订单占满批次；
-- 具体库存截止时间只保存在 orders.expires_at，支付尝试不再复制该字段。
CREATE INDEX payment_attempts_provider_state_updated_idx
ON payment_attempts(provider, state, updated_at);

-- 支付事件用于回调、主动查单等入口的幂等和审计；同一提供方事件只能入账一次。
CREATE TABLE payment_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider TEXT NOT NULL CHECK (provider IN ('epay', 'wechatpay')),
    provider_event_id TEXT NOT NULL,
    payment_attempt_id UUID NOT NULL,
    event_type TEXT NOT NULL,
    request_body TEXT NOT NULL DEFAULT '',
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
