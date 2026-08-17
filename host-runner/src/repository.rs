use std::collections::HashMap;

use chrono::{DateTime, Utc};
use postgres::{GenericClient, Row, error::SqlState};

use crate::{
    bindings::{inventory, order, product},
    db::HostState,
};

type Product = product::ehlung::commerce::models::Product;
type ProductDraft = product::ehlung::commerce::models::ProductDraft;
type InventoryProduct = inventory::ehlung::commerce::models::Product;
type Order = order::ehlung::commerce::models::Order;
type OrderInput = order::ehlung::commerce::models::OrderInput;
type OrderLine = order::ehlung::commerce::models::OrderLine;
type OrderQuote = order::ehlung::commerce::models::OrderQuote;
type OrderQuoteLine = order::ehlung::commerce::models::OrderQuoteLine;

impl product::ehlung::commerce::models::Host for HostState {}
impl inventory::ehlung::commerce::models::Host for HostState {}
impl order::ehlung::commerce::models::Host for HostState {}

impl product::ehlung::commerce::product_repository::Host for HostState {
    fn list(&mut self) -> Result<Vec<Product>, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        client
            .query(
                "SELECT id, sku, name, price_cents, stock, active, created_at, updated_at \
                 FROM rust_wasm_shop.products ORDER BY id",
                &[],
            )
            .map_err(db_error)?
            .iter()
            .map(product_from_row)
            .collect()
    }

    fn create(&mut self, draft: ProductDraft) -> Result<Product, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        let price = to_i64(draft.price_cents, "상품 가격")?;
        let stock = to_i64(draft.initial_stock, "초기 재고")?;
        let row = client
            .query_one(
                "INSERT INTO rust_wasm_shop.products (sku, name, price_cents, stock) \
                 VALUES ($1, $2, $3, $4) \
                 RETURNING id, sku, name, price_cents, stock, active, created_at, updated_at",
                &[&draft.sku, &draft.name, &price, &stock],
            )
            .map_err(|error| {
                if error.code() == Some(&SqlState::UNIQUE_VIOLATION) {
                    "이미 사용 중인 SKU입니다.".to_string()
                } else {
                    db_error(error)
                }
            })?;
        product_from_row(&row)
    }
}

impl inventory::ehlung::commerce::inventory_repository::Host for HostState {
    fn adjust_stock(&mut self, product_id: u64, delta: i64) -> Result<InventoryProduct, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        let id = to_i64(product_id, "상품 ID")?;
        let row = client
            .query_opt(
                "UPDATE rust_wasm_shop.products \
                 SET stock = stock + $2, updated_at = NOW() \
                 WHERE id = $1 AND active = TRUE AND stock + $2 >= 0 \
                 RETURNING id, sku, name, price_cents, stock, active, created_at, updated_at",
                &[&id, &delta],
            )
            .map_err(db_error)?
            .ok_or_else(|| "상품을 찾을 수 없거나 재고가 부족합니다.".to_string())?;

        inventory_product_from_row(&row)
    }
}

impl order::ehlung::commerce::order_repository::Host for HostState {
    fn quote(&mut self, input: OrderInput) -> Result<OrderQuote, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        build_quote(&mut *client, &input, false)
    }

    fn create(&mut self, input: OrderInput, expected_total_cents: u64) -> Result<Order, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        let mut transaction = client.transaction().map_err(db_error)?;
        let quote = build_quote(&mut transaction, &input, true)?;

        if quote.total_cents != expected_total_cents {
            return Err("견적 이후 상품 가격이 변경되었습니다. 다시 시도해주세요.".to_string());
        }

        let total = to_i64(quote.total_cents, "주문 합계")?;
        let order_row = transaction
            .query_one(
                "INSERT INTO rust_wasm_shop.orders (status, total_cents) VALUES ('CREATED', $1) \
                 RETURNING id, status, total_cents, created_at, updated_at",
                &[&total],
            )
            .map_err(db_error)?;
        let order_id: i64 = order_row.get("id");

        let product_ids: Vec<i64> = input
            .lines
            .iter()
            .map(|line| to_i64(line.product_id, "상품 ID"))
            .collect::<Result<_, _>>()?;
        let product_rows = transaction
            .query(
                "SELECT id, sku, name FROM rust_wasm_shop.products WHERE id = ANY($1)",
                &[&product_ids],
            )
            .map_err(db_error)?;
        let product_names: HashMap<i64, (String, String)> = product_rows
            .into_iter()
            .map(|row| (row.get("id"), (row.get("sku"), row.get("name"))))
            .collect();

        let mut lines = Vec::with_capacity(quote.lines.len());
        for quoted in &quote.lines {
            let product_id = to_i64(quoted.product_id, "상품 ID")?;
            let quantity = i32::try_from(quoted.quantity)
                .map_err(|_| "주문 수량이 너무 큽니다.".to_string())?;
            let unit_price = to_i64(quoted.unit_price_cents, "상품 가격")?;
            let line_total = unit_price
                .checked_mul(i64::from(quantity))
                .ok_or_else(|| "상품 금액 계산 범위를 초과했습니다.".to_string())?;
            let (sku, name) = product_names
                .get(&product_id)
                .cloned()
                .ok_or_else(|| "주문 상품 정보를 찾을 수 없습니다.".to_string())?;

            transaction
                .execute(
                    "UPDATE rust_wasm_shop.products \
                     SET stock = stock - $2, updated_at = NOW() WHERE id = $1",
                    &[&product_id, &i64::from(quantity)],
                )
                .map_err(db_error)?;
            transaction
                .execute(
                    "INSERT INTO rust_wasm_shop.order_items \
                     (order_id, product_id, sku, product_name, quantity, unit_price_cents, line_total_cents) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[&order_id, &product_id, &sku, &name, &quantity, &unit_price, &line_total],
                )
                .map_err(db_error)?;

            lines.push(OrderLine {
                product_id: quoted.product_id,
                sku,
                product_name: name,
                quantity: quoted.quantity,
                unit_price_cents: quoted.unit_price_cents,
                line_total_cents: u64::try_from(line_total).map_err(conversion_error)?,
            });
        }

        let result = order_from_header(&order_row, lines)?;
        transaction.commit().map_err(db_error)?;
        Ok(result)
    }

    fn get(&mut self, order_id: u64) -> Result<Option<Order>, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        load_order(&mut *client, order_id)
    }

    fn cancel(&mut self, order_id: u64) -> Result<Order, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        let mut transaction = client.transaction().map_err(db_error)?;
        let id = to_i64(order_id, "주문 ID")?;
        let status_row = transaction
            .query_opt(
                "SELECT status FROM rust_wasm_shop.orders WHERE id = $1 FOR UPDATE",
                &[&id],
            )
            .map_err(db_error)?
            .ok_or_else(|| "주문을 찾을 수 없습니다.".to_string())?;
        let status: String = status_row.get("status");

        if status != "CANCELLED" {
            let items = transaction
                .query(
                    "SELECT product_id, quantity FROM rust_wasm_shop.order_items WHERE order_id = $1",
                    &[&id],
                )
                .map_err(db_error)?;
            for item in items {
                let product_id: i64 = item.get("product_id");
                let quantity: i32 = item.get("quantity");
                transaction
                    .execute(
                        "UPDATE rust_wasm_shop.products \
                         SET stock = stock + $2, updated_at = NOW() WHERE id = $1",
                        &[&product_id, &i64::from(quantity)],
                    )
                    .map_err(db_error)?;
            }
            transaction
                .execute(
                    "UPDATE rust_wasm_shop.orders \
                     SET status = 'CANCELLED', updated_at = NOW() WHERE id = $1",
                    &[&id],
                )
                .map_err(db_error)?;
        }

        let result = load_order(&mut transaction, order_id)?
            .ok_or_else(|| "주문을 찾을 수 없습니다.".to_string())?;
        transaction.commit().map_err(db_error)?;
        Ok(result)
    }
}

fn build_quote<C: GenericClient>(
    client: &mut C,
    input: &OrderInput,
    lock: bool,
) -> Result<OrderQuote, String> {
    let product_ids: Vec<i64> = input
        .lines
        .iter()
        .map(|line| to_i64(line.product_id, "상품 ID"))
        .collect::<Result<_, _>>()?;
    let lock_clause = if lock { " FOR UPDATE" } else { "" };
    let query = format!(
        "SELECT id, price_cents, stock, active FROM rust_wasm_shop.products \
         WHERE id = ANY($1){lock_clause}"
    );
    let rows = client.query(&query, &[&product_ids]).map_err(db_error)?;
    let products: HashMap<i64, (i64, i64, bool)> = rows
        .into_iter()
        .map(|row| {
            (
                row.get("id"),
                (row.get("price_cents"), row.get("stock"), row.get("active")),
            )
        })
        .collect();

    let mut total = 0_u64;
    let mut lines = Vec::with_capacity(input.lines.len());
    for line in &input.lines {
        let product_id = to_i64(line.product_id, "상품 ID")?;
        let (price, stock, active) = products
            .get(&product_id)
            .copied()
            .ok_or_else(|| format!("상품 {}를 찾을 수 없습니다.", line.product_id))?;
        if !active {
            return Err(format!("상품 {}는 판매 중이 아닙니다.", line.product_id));
        }
        let price = u64::try_from(price).map_err(conversion_error)?;
        let stock = u64::try_from(stock).map_err(conversion_error)?;
        if u64::from(line.quantity) > stock {
            return Err(format!("상품 {}의 재고가 부족합니다.", line.product_id));
        }
        let line_total = price
            .checked_mul(u64::from(line.quantity))
            .ok_or_else(|| "상품 금액 계산 범위를 초과했습니다.".to_string())?;
        total = total
            .checked_add(line_total)
            .ok_or_else(|| "주문 합계 계산 범위를 초과했습니다.".to_string())?;
        lines.push(OrderQuoteLine {
            product_id: line.product_id,
            quantity: line.quantity,
            unit_price_cents: price,
            available_stock: stock,
        });
    }

    Ok(OrderQuote {
        lines,
        total_cents: total,
    })
}

fn load_order<C: GenericClient>(client: &mut C, order_id: u64) -> Result<Option<Order>, String> {
    let id = to_i64(order_id, "주문 ID")?;
    let Some(header) = client
        .query_opt(
            "SELECT id, status, total_cents, created_at, updated_at \
             FROM rust_wasm_shop.orders WHERE id = $1",
            &[&id],
        )
        .map_err(db_error)?
    else {
        return Ok(None);
    };
    let lines = client
        .query(
            "SELECT product_id, sku, product_name, quantity, unit_price_cents, line_total_cents \
             FROM rust_wasm_shop.order_items WHERE order_id = $1 ORDER BY product_id",
            &[&id],
        )
        .map_err(db_error)?
        .iter()
        .map(order_line_from_row)
        .collect::<Result<Vec<_>, _>>()?;

    order_from_header(&header, lines).map(Some)
}

fn product_from_row(row: &Row) -> Result<Product, String> {
    Ok(Product {
        id: row_u64(row, "id")?,
        sku: row.get("sku"),
        name: row.get("name"),
        price_cents: row_u64(row, "price_cents")?,
        stock: row_u64(row, "stock")?,
        active: row.get("active"),
        created_at: row_time(row, "created_at"),
        updated_at: row_time(row, "updated_at"),
    })
}

fn inventory_product_from_row(row: &Row) -> Result<InventoryProduct, String> {
    Ok(InventoryProduct {
        id: row_u64(row, "id")?,
        sku: row.get("sku"),
        name: row.get("name"),
        price_cents: row_u64(row, "price_cents")?,
        stock: row_u64(row, "stock")?,
        active: row.get("active"),
        created_at: row_time(row, "created_at"),
        updated_at: row_time(row, "updated_at"),
    })
}

fn order_line_from_row(row: &Row) -> Result<OrderLine, String> {
    let quantity: i32 = row.get("quantity");
    Ok(OrderLine {
        product_id: row_u64(row, "product_id")?,
        sku: row.get("sku"),
        product_name: row.get("product_name"),
        quantity: u32::try_from(quantity).map_err(conversion_error)?,
        unit_price_cents: row_u64(row, "unit_price_cents")?,
        line_total_cents: row_u64(row, "line_total_cents")?,
    })
}

fn order_from_header(row: &Row, lines: Vec<OrderLine>) -> Result<Order, String> {
    Ok(Order {
        id: row_u64(row, "id")?,
        status: row.get("status"),
        total_cents: row_u64(row, "total_cents")?,
        lines,
        created_at: row_time(row, "created_at"),
        updated_at: row_time(row, "updated_at"),
    })
}

fn row_u64(row: &Row, column: &str) -> Result<u64, String> {
    let value: i64 = row.get(column);
    u64::try_from(value).map_err(conversion_error)
}

fn row_time(row: &Row, column: &str) -> String {
    let value: DateTime<Utc> = row.get(column);
    value.to_rfc3339()
}

fn to_i64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} 값이 지원 범위를 초과했습니다."))
}

fn conversion_error(error: impl std::fmt::Display) -> String {
    format!("DB 숫자 변환에 실패했습니다: {error}")
}

fn db_error(error: impl std::fmt::Display) -> String {
    tracing::error!(error = %error, "database operation failed");
    "데이터베이스 작업에 실패했습니다.".to_string()
}
