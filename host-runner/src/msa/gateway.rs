use std::{env, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::Response,
    routing::any,
};
use reqwest::Client;
use wasmtime::{
    Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::{
    bindings::gateway,
    support::{ServiceError, address, blocking, component},
};

struct GatewayHost {
    wasi: WasiCtx,
    table: ResourceTable,
}
impl GatewayHost {
    fn new() -> Self {
        let mut wasi = WasiCtx::builder();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        Self {
            wasi: wasi.build(),
            table: ResourceTable::new(),
        }
    }
}
impl WasiView for GatewayHost {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

struct GatewayRuntime {
    engine: Engine,
    component: Component,
}
impl GatewayRuntime {
    fn load() -> Result<Self> {
        let engine = Engine::default();
        let component = component(&engine, "gateway_component.wasm")?;
        Ok(Self { engine, component })
    }
    fn resolve(
        &self,
        method: &str,
        path: &str,
    ) -> Result<gateway::exports::ehlung::commerce::gateway::RouteDecision> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
        let mut store = Store::new(&self.engine, GatewayHost::new());
        let bindings =
            gateway::GatewayComponent::instantiate(&mut store, &self.component, &linker)?;
        bindings
            .ehlung_commerce_gateway()
            .call_resolve(&mut store, method, path)
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct GatewayState {
    runtime: Arc<GatewayRuntime>,
    client: Client,
    catalog_url: String,
    order_url: String,
}

pub async fn run() -> Result<()> {
    let state = GatewayState {
        runtime: Arc::new(GatewayRuntime::load()?),
        client: Client::builder().timeout(Duration::from_secs(5)).build()?,
        catalog_url: service_url("CATALOG_URL", "http://127.0.0.1:8092"),
        order_url: service_url("ORDER_URL", "http://127.0.0.1:8093"),
    };
    let address = address("0.0.0.0:8091")?;
    let app = Router::new()
        .fallback(any(dispatch))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "API Gateway Service started");
    axum::serve(listener, app).await?;
    Ok(())
}

fn service_url(name: &str, fallback: &str) -> String {
    env::var(name)
        .unwrap_or_else(|_| fallback.to_string())
        .trim_end_matches('/')
        .to_string()
}

async fn dispatch(
    State(state): State<GatewayState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ServiceError> {
    let runtime = Arc::clone(&state.runtime);
    let method_name = method.as_str().to_string();
    let path = uri.path().to_string();
    let route = blocking(move || runtime.resolve(&method_name, &path)).await?;
    use gateway::exports::ehlung::commerce::gateway::{RouteAction, RouteTarget};

    if matches!(
        (&route.target, &route.action),
        (RouteTarget::Health, RouteAction::Health)
    ) {
        return health(&state).await;
    }
    let base = match route.target {
        RouteTarget::Product | RouteTarget::Inventory => &state.catalog_url,
        RouteTarget::Order => &state.order_url,
        _ => {
            return Err(ServiceError::new(
                StatusCode::NOT_FOUND,
                "요청한 API 경로를 찾을 수 없습니다.",
            ));
        }
    };
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    let url = format!("{base}{path_and_query}");
    let mut request = state.client.request(method, url).body(body.to_vec());
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    let response = request.send().await.map_err(|error| {
        tracing::error!(%error, "downstream service request failed");
        ServiceError::new(StatusCode::BAD_GATEWAY, "내부 서비스에 연결할 수 없습니다.")
    })?;
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let bytes = response.bytes().await.map_err(ServiceError::internal)?;
    let mut result = Response::builder().status(status);
    if let Some(value) = content_type {
        result = result.header(header::CONTENT_TYPE, value);
    }
    result
        .body(Body::from(bytes))
        .map_err(ServiceError::internal)
}

async fn health(state: &GatewayState) -> Result<Response, ServiceError> {
    let catalog = downstream_health(&state.client, &state.catalog_url).await;
    let order = downstream_health(&state.client, &state.order_url).await;
    let healthy = catalog && order;
    let status = if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = serde_json::json!({
        "status": if healthy { "ok" } else { "degraded" },
        "runtime": "Wasmtime Component Model",
        "services": { "gateway": true, "catalog": catalog, "order": order }
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .map_err(ServiceError::internal)
}

async fn downstream_health(client: &Client, base: &str) -> bool {
    client
        .get(format!("{base}/internal/health"))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}
