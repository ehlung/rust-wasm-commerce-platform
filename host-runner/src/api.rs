use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    bindings::{gateway, inventory, order, product},
    runtime::ComponentRuntime,
};

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<ComponentRuntime>,
}

pub async fn dispatch(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Result<Response, ApiError> {
    let runtime = Arc::clone(&state.runtime);
    let method_name = method.as_str().to_string();
    let path = uri.path().to_string();
    let route = blocking(move || runtime.resolve_route(&method_name, &path)).await?;

    use gateway::exports::ehlung::commerce::gateway::{RouteAction, RouteTarget};
    match (route.target, route.action) {
        (RouteTarget::Health, RouteAction::Health) => Ok(Json(json!({
            "status": "ok",
            "runtime": "Wasmtime Component Model",
            "components": ["gateway", "product-agent", "inventory-agent", "order-agent"],
            "databaseAccess": "Host-provided WIT capability"
        }))
        .into_response()),
        (RouteTarget::Product, RouteAction::List) => list_products(state).await,
        (RouteTarget::Product, RouteAction::Create) => create_product(state, &body).await,
        (RouteTarget::Inventory, RouteAction::Adjust) => {
            let id = required_id(route.resource_id)?;
            adjust_stock(state, id, &body).await
        }
        (RouteTarget::Order, RouteAction::Create) => create_order(state, &body).await,
        (RouteTarget::Order, RouteAction::Get) => {
            let id = required_id(route.resource_id)?;
            get_order(state, id).await
        }
        (RouteTarget::Order, RouteAction::Cancel) => {
            let id = required_id(route.resource_id)?;
            cancel_order(state, id).await
        }
        _ => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "요청한 API 경로를 찾을 수 없습니다.",
        )),
    }
}

async fn list_products(state: AppState) -> Result<Response, ApiError> {
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.list_products()).await?;
    let products = result.map_err(ApiError::service_unavailable)?;
    let response: Vec<ProductResponse> = products
        .into_iter()
        .map(ProductResponse::from_product)
        .collect();
    Ok(Json(response).into_response())
}

async fn create_product(state: AppState, body: &[u8]) -> Result<Response, ApiError> {
    let request: CreateProductRequest = parse_body(body)?;
    let draft = product::ehlung::commerce::models::ProductDraft {
        sku: request.sku,
        name: request.name,
        price_cents: request.price_cents,
        initial_stock: request.initial_stock,
    };
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.create_product(draft)).await?;
    let product = result.map_err(ApiError::bad_request)?;
    Ok((
        StatusCode::CREATED,
        Json(ProductResponse::from_product(product)),
    )
        .into_response())
}

async fn adjust_stock(state: AppState, product_id: u64, body: &[u8]) -> Result<Response, ApiError> {
    let request: AdjustStockRequest = parse_body(body)?;
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.adjust_stock(product_id, request.delta)).await?;
    let product = result.map_err(ApiError::conflict)?;
    Ok(Json(ProductResponse::from_inventory_product(product)).into_response())
}

async fn create_order(state: AppState, body: &[u8]) -> Result<Response, ApiError> {
    let request: CreateOrderRequest = parse_body(body)?;
    let input = order::ehlung::commerce::models::OrderInput {
        lines: request
            .lines
            .into_iter()
            .map(|line| order::ehlung::commerce::models::OrderLineInput {
                product_id: line.product_id,
                quantity: line.quantity,
            })
            .collect(),
    };
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.create_order(input)).await?;
    let order = result.map_err(ApiError::conflict)?;
    Ok((StatusCode::CREATED, Json(OrderResponse::from_order(order))).into_response())
}

async fn get_order(state: AppState, order_id: u64) -> Result<Response, ApiError> {
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.get_order(order_id)).await?;
    let order = result
        .map_err(ApiError::service_unavailable)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "주문을 찾을 수 없습니다."))?;
    Ok(Json(OrderResponse::from_order(order)).into_response())
}

async fn cancel_order(state: AppState, order_id: u64) -> Result<Response, ApiError> {
    let runtime = Arc::clone(&state.runtime);
    let result = blocking(move || runtime.cancel_order(order_id)).await?;
    let order = result.map_err(ApiError::conflict)?;
    Ok(Json(OrderResponse::from_order(order)).into_response())
}

async fn blocking<T>(
    operation: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn parse_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::bad_request("요청 JSON 형식이 올바르지 않습니다.".to_string()))
}

fn required_id(id: Option<u64>) -> Result<u64, ApiError> {
    id.ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "리소스 ID가 필요합니다."))
}

#[derive(Deserialize)]
struct CreateProductRequest {
    sku: String,
    name: String,
    #[serde(rename = "priceCents")]
    price_cents: u64,
    #[serde(rename = "initialStock")]
    initial_stock: u64,
}

#[derive(Deserialize)]
struct AdjustStockRequest {
    delta: i64,
}

#[derive(Deserialize)]
struct CreateOrderRequest {
    lines: Vec<CreateOrderLineRequest>,
}

#[derive(Deserialize)]
struct CreateOrderLineRequest {
    #[serde(rename = "productId")]
    product_id: u64,
    quantity: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductResponse {
    id: u64,
    sku: String,
    name: String,
    price_cents: u64,
    stock: u64,
    active: bool,
    created_at: String,
    updated_at: String,
}

impl ProductResponse {
    fn from_product(value: product::ehlung::commerce::models::Product) -> Self {
        Self {
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

    fn from_inventory_product(value: inventory::ehlung::commerce::models::Product) -> Self {
        Self {
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderResponse {
    id: u64,
    status: String,
    total_cents: u64,
    lines: Vec<OrderLineResponse>,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderLineResponse {
    product_id: u64,
    sku: String,
    product_name: String,
    quantity: u32,
    unit_price_cents: u64,
    line_total_cents: u64,
}

impl OrderResponse {
    fn from_order(value: order::ehlung::commerce::models::Order) -> Self {
        Self {
            id: value.id,
            status: value.status,
            total_cents: value.total_cents,
            lines: value
                .lines
                .into_iter()
                .map(|line| OrderLineResponse {
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
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: String) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn conflict(message: String) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    fn service_unavailable(message: String) -> Self {
        tracing::error!(%message, "component repository operation failed");
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    fn internal(message: String) -> Self {
        tracing::error!(%message, "host runner operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "서버 내부 처리에 실패했습니다.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
