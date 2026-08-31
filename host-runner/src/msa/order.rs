use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use postgres::{GenericClient, Row};
use reqwest::blocking::Client;
use wasmtime::{
    Engine, Store,
    component::{HasSelf, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::db::{DbPool, create_pool_from_env};

use super::{
    bindings::order,
    dto::{
        CreateOrderDto, ErrorBody, OrderDto, OrderLineDto, OrderLineInputDto, QuoteDto,
        ReservationDto,
    },
    support::{ServiceError, address, blocking, component},
};

type Order = order::ehlung::commerce::models::Order;
type OrderInput = order::ehlung::commerce::models::OrderInput;
type OrderLine = order::ehlung::commerce::models::OrderLine;
type OrderQuote = order::ehlung::commerce::models::OrderQuote;
type OrderQuoteLine = order::ehlung::commerce::models::OrderQuoteLine;

struct OrderHost {
    pool: DbPool,
    catalog_url: String,
    client: Client,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl OrderHost {
    fn new(pool: DbPool, catalog_url: String, client: Client) -> Self {
        let mut wasi = WasiCtx::builder();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        Self {
            pool,
            catalog_url,
            client,
            wasi: wasi.build(),
            table: ResourceTable::new(),
        }
    }

    fn catalog_quote(&self, input: &OrderInput) -> Result<QuoteDto, String> {
        let lines: Vec<OrderLineInputDto> = input
            .lines
            .iter()
            .map(|line| OrderLineInputDto {
                product_id: line.product_id,
                quantity: line.quantity,
            })
            .collect();
        send_json(
            self.client
                .post(format!("{}/internal/quotes", self.catalog_url))
                .json(&lines),
        )
    }
}

impl WasiView for OrderHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl order::ehlung::commerce::models::Host for OrderHost {}

impl order::ehlung::commerce::order_repository::Host for OrderHost {
    fn quote(&mut self, input: OrderInput) -> Result<OrderQuote, String> {
        let quote = self.catalog_quote(&input)?;
        Ok(OrderQuote {
            total_cents: quote.total_cents,
            lines: quote
                .lines
                .into_iter()
                .map(|line| OrderQuoteLine {
                    product_id: line.product_id,
                    quantity: line.quantity,
                    unit_price_cents: line.unit_price_cents,
                    available_stock: line.available_stock,
                })
                .collect(),
        })
    }

    fn create(&mut self, input: OrderInput, expected_total_cents: u64) -> Result<Order, String> {
        let quote = self.catalog_quote(&input)?;
        if quote.total_cents != expected_total_cents {
            return Err("견적 이후 상품 가격이 변경되었습니다. 다시 시도해주세요.".to_string());
        }

        let mut db = self.pool.get().map_err(db_error)?;
        let mut tx = db.transaction().map_err(db_error)?;
        let total = to_i64(quote.total_cents, "주문 합계")?;
        let header = tx.query_one(
            "INSERT INTO rust_wasm_ordering.orders (status, total_cents) VALUES ('PENDING', $1) \
             RETURNING id, status, total_cents, created_at, updated_at", &[&total]
        ).map_err(db_error)?;
        let order_id: i64 = header.get("id");
        for line in &quote.lines {
            let product_id = to_i64(line.product_id, "상품 ID")?;
            let quantity =
                i32::try_from(line.quantity).map_err(|_| "주문 수량이 너무 큽니다.".to_string())?;
            let unit_price = to_i64(line.unit_price_cents, "상품 가격")?;
            let line_total = unit_price
                .checked_mul(i64::from(quantity))
                .ok_or_else(|| "상품 금액 계산 범위를 초과했습니다.".to_string())?;
            tx.execute(
                "INSERT INTO rust_wasm_ordering.order_items \
                 (order_id, product_id, sku, product_name, quantity, unit_price_cents, line_total_cents) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&order_id, &product_id, &line.sku, &line.product_name, &quantity, &unit_price, &line_total],
            ).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;

        let token = format!("order-{order_id}");
        let reservation = ReservationDto {
            token: token.clone(),
            lines: input
                .lines
                .iter()
                .map(|line| OrderLineInputDto {
                    product_id: line.product_id,
                    quantity: line.quantity,
                })
                .collect(),
        };
        let reserve_result = self
            .client
            .post(format!("{}/internal/reservations", self.catalog_url))
            .json(&reservation)
            .send();
        match reserve_result {
            Ok(response) if response.status().is_success() => {
                let mut db = self.pool.get().map_err(db_error)?;
                db.execute("UPDATE rust_wasm_ordering.orders SET status = 'CREATED', reservation_token = $2, updated_at = NOW() WHERE id = $1", &[&order_id, &token]).map_err(db_error)?;
            }
            Ok(response) => {
                let message = response
                    .json::<ErrorBody>()
                    .map(|body| body.error)
                    .unwrap_or_else(|_| "재고 예약에 실패했습니다.".to_string());
                let mut db = self.pool.get().map_err(db_error)?;
                db.execute("UPDATE rust_wasm_ordering.orders SET status = 'REJECTED', updated_at = NOW() WHERE id = $1", &[&order_id]).map_err(db_error)?;
                return Err(message);
            }
            Err(error) => {
                tracing::error!(%error, order_id, "catalog reservation result is uncertain");
                return Err(
                    "재고 서비스 응답을 확인하지 못했습니다. 주문 상태를 다시 조회해주세요."
                        .to_string(),
                );
            }
        }
        let mut db = self.pool.get().map_err(db_error)?;
        load_order(&mut *db, u64::try_from(order_id).map_err(conversion_error)?)?
            .ok_or_else(|| "생성한 주문을 찾을 수 없습니다.".to_string())
    }

    fn get(&mut self, order_id: u64) -> Result<Option<Order>, String> {
        let mut db = self.pool.get().map_err(db_error)?;
        load_order(&mut *db, order_id)
    }

    fn cancel(&mut self, order_id: u64) -> Result<Order, String> {
        let id = to_i64(order_id, "주문 ID")?;
        let token = {
            let mut db = self.pool.get().map_err(db_error)?;
            let mut tx = db.transaction().map_err(db_error)?;
            let row = tx.query_opt("SELECT status, reservation_token FROM rust_wasm_ordering.orders WHERE id = $1 FOR UPDATE", &[&id])
                .map_err(db_error)?.ok_or_else(|| "주문을 찾을 수 없습니다.".to_string())?;
            let status: String = row.get("status");
            if status == "CANCELLED" {
                tx.commit().map_err(db_error)?;
                let mut db = self.pool.get().map_err(db_error)?;
                return load_order(&mut *db, order_id)?
                    .ok_or_else(|| "주문을 찾을 수 없습니다.".to_string());
            }
            if status != "CREATED" && status != "CANCEL_PENDING" {
                return Err(format!("{status} 상태의 주문은 취소할 수 없습니다."));
            }
            let token: String = row
                .get::<_, Option<String>>("reservation_token")
                .ok_or_else(|| "재고 예약 정보를 찾을 수 없습니다.".to_string())?;
            tx.execute("UPDATE rust_wasm_ordering.orders SET status = 'CANCEL_PENDING', updated_at = NOW() WHERE id = $1", &[&id]).map_err(db_error)?;
            tx.commit().map_err(db_error)?;
            token
        };

        let response = self
            .client
            .post(format!(
                "{}/internal/reservations/{}/release",
                self.catalog_url, token
            ))
            .send()
            .map_err(|error| {
                tracing::error!(%error, order_id, "catalog release result is uncertain");
                "재고 서비스 응답을 확인하지 못했습니다. 주문 상태를 다시 조회해주세요.".to_string()
            })?;
        if !response.status().is_success() {
            return Err(response
                .json::<ErrorBody>()
                .map(|body| body.error)
                .unwrap_or_else(|_| "재고 예약 해제에 실패했습니다.".to_string()));
        }
        let mut db = self.pool.get().map_err(db_error)?;
        db.execute("UPDATE rust_wasm_ordering.orders SET status = 'CANCELLED', updated_at = NOW() WHERE id = $1", &[&id]).map_err(db_error)?;
        load_order(&mut *db, order_id)?.ok_or_else(|| "주문을 찾을 수 없습니다.".to_string())
    }
}

struct OrderRuntime {
    engine: Engine,
    component: wasmtime::component::Component,
    pool: DbPool,
    catalog_url: String,
    client: Client,
}

impl OrderRuntime {
    fn load(pool: DbPool, catalog_url: String) -> Result<Self> {
        let engine = Engine::default();
        let component = component(&engine, "order_agent_component.wasm")?;
        let client = Client::builder().timeout(Duration::from_secs(3)).build()?;
        Ok(Self {
            engine,
            component,
            pool,
            catalog_url,
            client,
        })
    }

    fn linker(&self) -> Result<Linker<OrderHost>> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        order::OrderComponent::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    fn store(&self) -> Store<OrderHost> {
        Store::new(
            &self.engine,
            OrderHost::new(
                self.pool.clone(),
                self.catalog_url.clone(),
                self.client.clone(),
            ),
        )
    }

    fn create(&self, input: OrderInput) -> Result<Result<Order, String>> {
        let linker = self.linker()?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.component, &linker)?;
        bindings
            .ehlung_commerce_order_agent()
            .call_create(&mut store, &input)
            .map_err(Into::into)
    }
    fn get(&self, id: u64) -> Result<Result<Option<Order>, String>> {
        let linker = self.linker()?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.component, &linker)?;
        bindings
            .ehlung_commerce_order_agent()
            .call_get(&mut store, id)
            .map_err(Into::into)
    }
    fn cancel(&self, id: u64) -> Result<Result<Order, String>> {
        let linker = self.linker()?;
        let mut store = self.store();
        let bindings = order::OrderComponent::instantiate(&mut store, &self.component, &linker)?;
        bindings
            .ehlung_commerce_order_agent()
            .call_cancel(&mut store, id)
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct OrderState {
    runtime: Arc<OrderRuntime>,
}

pub async fn run() -> Result<()> {
    let catalog_url = env::var("CATALOG_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8092".to_string())
        .trim_end_matches('/')
        .to_string();
    let state = tokio::task::spawn_blocking(move || -> Result<OrderState> {
        let pool = create_pool_from_env("ORDER_DATABASE_URL")?;
        pool.get()
            .context("Order Service가 PostgreSQL에 연결하지 못했습니다")?;
        Ok(OrderState {
            runtime: Arc::new(OrderRuntime::load(pool, catalog_url)?),
        })
    })
    .await
    .context("Order Service 초기화 작업이 비정상 종료되었습니다")??;
    let address = address("127.0.0.1:8093")?;
    let app = Router::new()
        .route("/internal/health", get(health))
        .route("/api/orders", post(create_order))
        .route("/api/orders/{id}", get(get_order))
        .route("/api/orders/{id}/cancel", post(cancel_order))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Order Service started");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok","service":"order","components":["order-agent"]}))
}

async fn create_order(
    State(state): State<OrderState>,
    Json(request): Json<CreateOrderDto>,
) -> Result<(StatusCode, Json<OrderDto>), ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let input = OrderInput {
        lines: request
            .lines
            .into_iter()
            .map(|line| order::ehlung::commerce::models::OrderLineInput {
                product_id: line.product_id,
                quantity: line.quantity,
            })
            .collect(),
    };
    let value = blocking(move || runtime.create(input))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok((StatusCode::CREATED, Json(order_dto(value))))
}

async fn get_order(
    State(state): State<OrderState>,
    Path(id): Path<u64>,
) -> Result<Json<OrderDto>, ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let value = blocking(move || runtime.get(id))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::SERVICE_UNAVAILABLE, message))?
        .ok_or_else(|| ServiceError::new(StatusCode::NOT_FOUND, "주문을 찾을 수 없습니다."))?;
    Ok(Json(order_dto(value)))
}

async fn cancel_order(
    State(state): State<OrderState>,
    Path(id): Path<u64>,
) -> Result<Json<OrderDto>, ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let value = blocking(move || runtime.cancel(id))
        .await?
        .map_err(|message| ServiceError::new(StatusCode::CONFLICT, message))?;
    Ok(Json(order_dto(value)))
}

fn send_json<T: serde::de::DeserializeOwned>(
    builder: reqwest::blocking::RequestBuilder,
) -> Result<T, String> {
    let response = builder.send().map_err(|error| {
        tracing::error!(%error, "catalog request failed");
        "Catalog Service에 연결할 수 없습니다.".to_string()
    })?;
    if response.status().is_success() {
        response
            .json()
            .map_err(|_| "Catalog Service 응답을 해석할 수 없습니다.".to_string())
    } else {
        Err(response
            .json::<ErrorBody>()
            .map(|body| body.error)
            .unwrap_or_else(|_| "Catalog Service 요청에 실패했습니다.".to_string()))
    }
}

fn load_order<C: GenericClient>(db: &mut C, order_id: u64) -> Result<Option<Order>, String> {
    let id = to_i64(order_id, "주문 ID")?;
    let Some(header) = db.query_opt("SELECT id, status, total_cents, created_at, updated_at FROM rust_wasm_ordering.orders WHERE id = $1", &[&id]).map_err(db_error)? else { return Ok(None); };
    let lines = db.query("SELECT product_id, sku, product_name, quantity, unit_price_cents, line_total_cents FROM rust_wasm_ordering.order_items WHERE order_id = $1 ORDER BY product_id", &[&id]).map_err(db_error)?
        .iter().map(order_line_from_row).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(Order {
        id: row_u64(&header, "id")?,
        status: header.get("status"),
        total_cents: row_u64(&header, "total_cents")?,
        lines,
        created_at: row_time(&header, "created_at"),
        updated_at: row_time(&header, "updated_at"),
    }))
}

fn order_line_from_row(row: &Row) -> Result<OrderLine, String> {
    Ok(OrderLine {
        product_id: row_u64(row, "product_id")?,
        sku: row.get("sku"),
        product_name: row.get("product_name"),
        quantity: u32::try_from(row.get::<_, i32>("quantity")).map_err(conversion_error)?,
        unit_price_cents: row_u64(row, "unit_price_cents")?,
        line_total_cents: row_u64(row, "line_total_cents")?,
    })
}
fn order_dto(value: Order) -> OrderDto {
    OrderDto {
        id: value.id,
        status: value.status,
        total_cents: value.total_cents,
        lines: value
            .lines
            .into_iter()
            .map(|line| OrderLineDto {
                product_id: line.product_id,
                sku: line.sku,
                product_name: line.product_name,
                quantity: line.quantity,
                unit_price_cents: line.unit_price_cents,
                line_total_cents: line.line_total_cents,
            })
            .collect(),
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}
fn row_u64(row: &Row, column: &str) -> Result<u64, String> {
    u64::try_from(row.get::<_, i64>(column)).map_err(conversion_error)
}
fn row_time(row: &Row, column: &str) -> String {
    row.get::<_, DateTime<Utc>>(column).to_rfc3339()
}
fn to_i64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} 값이 지원 범위를 초과했습니다."))
}
fn conversion_error(error: impl std::fmt::Display) -> String {
    format!("DB 숫자 변환에 실패했습니다: {error}")
}
fn db_error(error: impl std::fmt::Display) -> String {
    tracing::error!(%error, "order database operation failed");
    "데이터베이스 작업에 실패했습니다.".to_string()
}
