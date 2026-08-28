use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use postgres::{Row, error::SqlState};
use wasmtime::component::ResourceTable;
use wasmtime::{
    Engine, Store,
    component::{HasSelf, Linker},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::db::{DbPool, create_pool};

use super::{
    bindings::{inventory, product},
    dto::{
        AdjustStockDto, CreateProductDto, OrderLineInputDto, ProductDto, QuoteDto, QuoteLineDto,
        ReservationDto,
    },
    support::{ServiceError, address, blocking, component},
};

type Product = product::ehlung::commerce::models::Product;
type ProductDraft = product::ehlung::commerce::models::ProductDraft;
type InventoryProduct = inventory::ehlung::commerce::models::Product;

struct CatalogHost {
    pool: DbPool,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl CatalogHost {
    fn new(pool: DbPool) -> Self {
        let mut wasi = WasiCtx::builder();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        Self {
            pool,
            wasi: wasi.build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for CatalogHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl product::ehlung::commerce::models::Host for CatalogHost {}
impl inventory::ehlung::commerce::models::Host for CatalogHost {}

impl product::ehlung::commerce::product_repository::Host for CatalogHost {
    fn list(&mut self) -> Result<Vec<Product>, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        client
            .query(
                "SELECT id, sku, name, price_cents, stock, active, created_at, updated_at \
             FROM rust_wasm_catalog.products ORDER BY id",
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
                "INSERT INTO rust_wasm_catalog.products (sku, name, price_cents, stock) \
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

impl inventory::ehlung::commerce::inventory_repository::Host for CatalogHost {
    fn adjust_stock(&mut self, product_id: u64, delta: i64) -> Result<InventoryProduct, String> {
        let mut client = self.pool.get().map_err(db_error)?;
        let id = to_i64(product_id, "상품 ID")?;
        let row = client
            .query_opt(
                "UPDATE rust_wasm_catalog.products SET stock = stock + $2, updated_at = NOW() \
             WHERE id = $1 AND active = TRUE AND stock + $2 >= 0 \
             RETURNING id, sku, name, price_cents, stock, active, created_at, updated_at",
                &[&id, &delta],
            )
            .map_err(db_error)?
            .ok_or_else(|| "상품을 찾을 수 없거나 재고가 부족합니다.".to_string())?;
        inventory_product_from_row(&row)
    }
}

struct CatalogRuntime {
    engine: Engine,
    product: wasmtime::component::Component,
    inventory: wasmtime::component::Component,
    pool: DbPool,
}

impl CatalogRuntime {
    fn load(pool: DbPool) -> Result<Self> {
        let engine = Engine::default();
        Ok(Self {
            product: component(&engine, "product_agent_component.wasm")?,
            inventory: component(&engine, "inventory_agent_component.wasm")?,
            engine,
            pool,
        })
    }

    fn product_linker(&self) -> Result<Linker<CatalogHost>> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        product::ProductComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    fn inventory_linker(&self) -> Result<Linker<CatalogHost>> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        inventory::InventoryComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    fn list_products(&self) -> Result<Result<Vec<Product>, String>> {
        let linker = self.product_linker()?;
        let mut store = Store::new(&self.engine, CatalogHost::new(self.pool.clone()));
        let bindings = product::ProductComponent::instantiate(&mut store, &self.product, &linker)?;
        bindings
            .ehlung_commerce_product_agent()
            .call_list(&mut store)
            .map_err(Into::into)
    }

    fn create_product(&self, draft: ProductDraft) -> Result<Result<Product, String>> {
        let linker = self.product_linker()?;
        let mut store = Store::new(&self.engine, CatalogHost::new(self.pool.clone()));
        let bindings = product::ProductComponent::instantiate(&mut store, &self.product, &linker)?;
        bindings
            .ehlung_commerce_product_agent()
            .call_create(&mut store, &draft)
            .map_err(Into::into)
    }

    fn adjust_stock(
        &self,
        product_id: u64,
        delta: i64,
    ) -> Result<Result<InventoryProduct, String>> {
        let linker = self.inventory_linker()?;
        let mut store = Store::new(&self.engine, CatalogHost::new(self.pool.clone()));
        let bindings =
            inventory::InventoryComponent::instantiate(&mut store, &self.inventory, &linker)?;
        bindings
            .ehlung_commerce_inventory_agent()
            .call_adjust_stock(&mut store, product_id, delta)
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct CatalogState {
    runtime: Arc<CatalogRuntime>,
    pool: DbPool,
}

pub async fn run() -> Result<()> {
    let pool = create_pool()?;
    pool.get()
        .context("Catalog Service가 PostgreSQL에 연결하지 못했습니다")?;
    let runtime = CatalogRuntime::load(pool.clone())?;
    let state = CatalogState {
        runtime: Arc::new(runtime),
        pool,
    };
    let address = address("127.0.0.1:8092")?;
    let app = Router::new()
        .route("/internal/health", get(health))
        .route("/api/products", get(list_products).post(create_product))
        .route("/api/products/{id}/stock", patch(adjust_stock))
        .route("/internal/quotes", post(quote))
        .route("/internal/reservations", post(reserve))
        .route("/internal/reservations/{token}/release", post(release))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Catalog Service started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"status":"ok","service":"catalog","components":["product-agent","inventory-agent"]}),
    )
}

async fn list_products(
    State(state): State<CatalogState>,
) -> Result<Json<Vec<ProductDto>>, ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let values = blocking(move || runtime.list_products())
        .await?
        .map_err(|message| ServiceError::new(StatusCode::SERVICE_UNAVAILABLE, message))?;
    Ok(Json(values.into_iter().map(product_dto).collect()))
}

async fn create_product(
    State(state): State<CatalogState>,
    Json(request): Json<CreateProductDto>,
) -> Result<(StatusCode, Json<ProductDto>), ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let draft = ProductDraft {
        sku: request.sku,
        name: request.name,
        price_cents: request.price_cents,
        initial_stock: request.initial_stock,
    };
    let value = blocking(move || runtime.create_product(draft))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::BAD_REQUEST, message))?;
    Ok((StatusCode::CREATED, Json(product_dto(value))))
}

async fn adjust_stock(
    State(state): State<CatalogState>,
    Path(id): Path<u64>,
    Json(request): Json<AdjustStockDto>,
) -> Result<Json<ProductDto>, ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let value = blocking(move || runtime.adjust_stock(id, request.delta))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok(Json(inventory_product_dto(value)))
}

async fn quote(
    State(state): State<CatalogState>,
    Json(lines): Json<Vec<OrderLineInputDto>>,
) -> Result<Json<QuoteDto>, ServiceError> {
    let pool = state.pool.clone();
    let quote = blocking(move || build_quote(&pool, &lines))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok(Json(quote))
}

async fn reserve(
    State(state): State<CatalogState>,
    Json(request): Json<ReservationDto>,
) -> Result<StatusCode, ServiceError> {
    let pool = state.pool.clone();
    blocking(move || reserve_stock(&pool, &request))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn release(
    State(state): State<CatalogState>,
    Path(token): Path<String>,
) -> Result<StatusCode, ServiceError> {
    let pool = state.pool.clone();
    blocking(move || release_stock(&pool, &token))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok(StatusCode::NO_CONTENT)
}

fn build_quote(pool: &DbPool, lines: &[OrderLineInputDto]) -> Result<Result<QuoteDto, String>> {
    let mut client = pool.get()?;
    let mut total = 0_u64;
    let mut quoted = Vec::with_capacity(lines.len());
    for line in lines {
        let id = to_i64(line.product_id, "상품 ID").map_err(anyhow::Error::msg)?;
        let row = match client.query_opt(
            "SELECT id, sku, name, price_cents, stock, active FROM rust_wasm_catalog.products WHERE id = $1", &[&id]
        )? {
            Some(row) => row,
            None => return Ok(Err(format!("상품 {}를 찾을 수 없습니다.", line.product_id))),
        };
        if !row.get::<_, bool>("active") {
            return Ok(Err(format!(
                "상품 {}는 판매 중이 아닙니다.",
                line.product_id
            )));
        }
        let stock = row_u64(&row, "stock").map_err(anyhow::Error::msg)?;
        if u64::from(line.quantity) > stock {
            return Ok(Err(format!(
                "상품 {}의 재고가 부족합니다.",
                line.product_id
            )));
        }
        let price = row_u64(&row, "price_cents").map_err(anyhow::Error::msg)?;
        total = total
            .checked_add(
                price
                    .checked_mul(u64::from(line.quantity))
                    .ok_or_else(|| anyhow!("상품 금액 계산 범위를 초과했습니다"))?,
            )
            .ok_or_else(|| anyhow!("주문 합계 계산 범위를 초과했습니다"))?;
        quoted.push(QuoteLineDto {
            product_id: line.product_id,
            sku: row.get("sku"),
            product_name: row.get("name"),
            quantity: line.quantity,
            unit_price_cents: price,
            available_stock: stock,
        });
    }
    Ok(Ok(QuoteDto {
        lines: quoted,
        total_cents: total,
    }))
}

fn reserve_stock(pool: &DbPool, request: &ReservationDto) -> Result<Result<(), String>> {
    let mut client = pool.get()?;
    let mut tx = client.transaction()?;
    let existing = tx.query(
        "SELECT product_id, quantity, status FROM rust_wasm_catalog.inventory_reservations WHERE token = $1 ORDER BY product_id FOR UPDATE",
        &[&request.token],
    )?;
    if !existing.is_empty() {
        let expected: std::collections::BTreeMap<i64, i32> = request
            .lines
            .iter()
            .map(|line| {
                Ok((
                    to_i64(line.product_id, "상품 ID")?,
                    i32::try_from(line.quantity)
                        .map_err(|_| "주문 수량이 너무 큽니다.".to_string())?,
                ))
            })
            .collect::<Result<_, String>>()
            .map_err(anyhow::Error::msg)?;
        let actual: std::collections::BTreeMap<i64, i32> = existing
            .iter()
            .map(|row| (row.get("product_id"), row.get("quantity")))
            .collect();
        let all_reserved = existing
            .iter()
            .all(|row| row.get::<_, String>("status") == "RESERVED");
        return Ok(if expected == actual && all_reserved {
            Ok(())
        } else {
            Err("예약 토큰이 이미 다른 요청에 사용되었습니다.".to_string())
        });
    }

    let mut lines = request.lines.clone();
    lines.sort_by_key(|line| line.product_id);
    for line in lines {
        let id = to_i64(line.product_id, "상품 ID").map_err(anyhow::Error::msg)?;
        let quantity =
            i32::try_from(line.quantity).map_err(|_| anyhow!("주문 수량이 너무 큽니다"))?;
        let updated = tx.execute(
            "UPDATE rust_wasm_catalog.products SET stock = stock - $2, updated_at = NOW() \
             WHERE id = $1 AND active = TRUE AND stock >= $2",
            &[&id, &i64::from(quantity)],
        )?;
        if updated != 1 {
            return Ok(Err(format!(
                "상품 {}의 재고가 부족합니다.",
                line.product_id
            )));
        }
        tx.execute(
            "INSERT INTO rust_wasm_catalog.inventory_reservations (token, product_id, quantity, status) VALUES ($1, $2, $3, 'RESERVED')",
            &[&request.token, &id, &quantity],
        )?;
    }
    tx.commit()?;
    Ok(Ok(()))
}

fn release_stock(pool: &DbPool, token: &str) -> Result<Result<(), String>> {
    let mut client = pool.get()?;
    let mut tx = client.transaction()?;
    let rows = tx.query(
        "SELECT product_id, quantity, status FROM rust_wasm_catalog.inventory_reservations WHERE token = $1 ORDER BY product_id FOR UPDATE",
        &[&token],
    )?;
    if rows.is_empty() {
        return Ok(Err("재고 예약을 찾을 수 없습니다.".to_string()));
    }
    for row in &rows {
        if row.get::<_, String>("status") == "RESERVED" {
            let product_id: i64 = row.get("product_id");
            let quantity: i32 = row.get("quantity");
            tx.execute("UPDATE rust_wasm_catalog.products SET stock = stock + $2, updated_at = NOW() WHERE id = $1", &[&product_id, &i64::from(quantity)])?;
        }
    }
    tx.execute("UPDATE rust_wasm_catalog.inventory_reservations SET status = 'RELEASED', updated_at = NOW() WHERE token = $1 AND status = 'RESERVED'", &[&token])?;
    tx.commit()?;
    Ok(Ok(()))
}

fn product_dto(value: Product) -> ProductDto {
    ProductDto {
        id: value.id,
        sku: value.sku,
        name: value.name,
        price_cents: value.price_cents,
        stock: value.stock,
        active: value.active,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}

fn inventory_product_dto(value: InventoryProduct) -> ProductDto {
    ProductDto {
        id: value.id,
        sku: value.sku,
        name: value.name,
        price_cents: value.price_cents,
        stock: value.stock,
        active: value.active,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
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

fn row_u64(row: &Row, column: &str) -> Result<u64, String> {
    u64::try_from(row.get::<_, i64>(column))
        .map_err(|error| format!("DB 숫자 변환에 실패했습니다: {error}"))
}
fn row_time(row: &Row, column: &str) -> String {
    row.get::<_, DateTime<Utc>>(column).to_rfc3339()
}
fn to_i64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} 값이 지원 범위를 초과했습니다."))
}
fn db_error(error: impl std::fmt::Display) -> String {
    tracing::error!(%error, "catalog database operation failed");
    "데이터베이스 작업에 실패했습니다.".to_string()
}
