CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE product_info (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    image_base64 TEXT,
    name TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '',
    price_cents BIGINT NOT NULL CHECK (price_cents >= 0),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_info_id UUID NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'available' CHECK (status IN ('available', 'reserved', 'delivered', 'disabled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX products_info_status_idx ON products(product_info_id, status);
CREATE INDEX products_created_at_id_idx ON products(created_at DESC, id DESC);

CREATE TABLE site_settings (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    order_allocation_mode TEXT NOT NULL DEFAULT 'reserve_on_create' CHECK (order_allocation_mode IN ('reserve_on_create', 'allocate_on_pay')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO site_settings (id, order_allocation_mode)
VALUES (TRUE, 'reserve_on_create');

CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    epay_trade_no TEXT NOT NULL UNIQUE,
    product_id UUID,
    product_info_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid', 'preorder')),
    contact TEXT NOT NULL,
    order_password_hash TEXT NOT NULL
);

CREATE INDEX orders_created_at_id_idx ON orders(created_at DESC, id DESC);
CREATE INDEX orders_contact_created_at_id_idx ON orders(contact, created_at DESC, id DESC);
CREATE INDEX orders_status_idx ON orders(status);
CREATE INDEX orders_product_info_status_idx ON orders(product_info_id, status);
CREATE INDEX orders_preorder_product_paid_idx ON orders(product_info_id, paid_at ASC, created_at ASC, id ASC)
WHERE status = 'preorder';

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

CREATE INDEX api_call_logs_api_name_created_at_idx ON api_call_logs(api_name, created_at DESC);
CREATE INDEX api_call_logs_created_at_id_idx ON api_call_logs(created_at DESC, id DESC);
