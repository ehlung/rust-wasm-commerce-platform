BEGIN;

CREATE SCHEMA IF NOT EXISTS rust_wasm_shop;

CREATE TABLE IF NOT EXISTS rust_wasm_shop.products (
    id BIGSERIAL PRIMARY KEY,
    sku VARCHAR(40) NOT NULL UNIQUE,
    name VARCHAR(120) NOT NULL,
    price_cents BIGINT NOT NULL CHECK (price_cents > 0),
    stock BIGINT NOT NULL DEFAULT 0 CHECK (stock >= 0),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rust_wasm_shop.orders (
    id BIGSERIAL PRIMARY KEY,
    status VARCHAR(20) NOT NULL CHECK (status IN ('CREATED', 'CANCELLED')),
    total_cents BIGINT NOT NULL CHECK (total_cents >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rust_wasm_shop.order_items (
    order_id BIGINT NOT NULL REFERENCES rust_wasm_shop.orders(id),
    product_id BIGINT NOT NULL REFERENCES rust_wasm_shop.products(id),
    sku VARCHAR(40) NOT NULL,
    product_name VARCHAR(120) NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    unit_price_cents BIGINT NOT NULL CHECK (unit_price_cents > 0),
    line_total_cents BIGINT NOT NULL CHECK (line_total_cents > 0),
    PRIMARY KEY (order_id, product_id)
);

CREATE INDEX IF NOT EXISTS idx_orders_created_at
    ON rust_wasm_shop.orders (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_order_items_product_id
    ON rust_wasm_shop.order_items (product_id);

COMMIT;

