CREATE SCHEMA IF NOT EXISTS rust_wasm_catalog;
CREATE SCHEMA IF NOT EXISTS rust_wasm_ordering;

CREATE TABLE IF NOT EXISTS rust_wasm_catalog.products (
    id BIGSERIAL PRIMARY KEY,
    sku VARCHAR(40) NOT NULL UNIQUE,
    name VARCHAR(120) NOT NULL,
    price_cents BIGINT NOT NULL CHECK (price_cents > 0),
    stock BIGINT NOT NULL DEFAULT 0 CHECK (stock >= 0),
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rust_wasm_catalog.inventory_reservations (
    token VARCHAR(100) NOT NULL,
    product_id BIGINT NOT NULL REFERENCES rust_wasm_catalog.products(id),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    status VARCHAR(20) NOT NULL CHECK (status IN ('RESERVED', 'RELEASED')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (token, product_id)
);

CREATE TABLE IF NOT EXISTS rust_wasm_ordering.orders (
    id BIGSERIAL PRIMARY KEY,
    status VARCHAR(20) NOT NULL CHECK (
        status IN ('PENDING', 'CREATED', 'REJECTED', 'CANCEL_PENDING', 'CANCELLED')
    ),
    total_cents BIGINT NOT NULL CHECK (total_cents >= 0),
    reservation_token VARCHAR(100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rust_wasm_ordering.order_items (
    id BIGSERIAL PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES rust_wasm_ordering.orders(id) ON DELETE CASCADE,
    product_id BIGINT NOT NULL,
    sku VARCHAR(40) NOT NULL,
    product_name VARCHAR(120) NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    unit_price_cents BIGINT NOT NULL CHECK (unit_price_cents > 0),
    line_total_cents BIGINT NOT NULL CHECK (line_total_cents > 0),
    UNIQUE (order_id, product_id)
);

CREATE INDEX IF NOT EXISTS idx_catalog_products_active ON rust_wasm_catalog.products(active);
CREATE INDEX IF NOT EXISTS idx_catalog_reservations_status ON rust_wasm_catalog.inventory_reservations(status);
CREATE INDEX IF NOT EXISTS idx_ordering_orders_status ON rust_wasm_ordering.orders(status);
CREATE INDEX IF NOT EXISTS idx_ordering_order_items_order_id ON rust_wasm_ordering.order_items(order_id);

COMMENT ON SCHEMA rust_wasm_catalog IS 'Catalog Service 전용 논리 스키마';
COMMENT ON SCHEMA rust_wasm_ordering IS 'Order Service 전용 논리 스키마';
COMMENT ON COLUMN rust_wasm_ordering.order_items.product_id IS 'Catalog Service 식별자 스냅샷이며 서비스 간 FK를 만들지 않음';
