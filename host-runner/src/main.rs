use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{Router, routing::any};
use tracing_subscriber::EnvFilter;

mod api;
mod bindings;
mod db;
mod msa;
mod repository;
mod runtime;

use api::{AppState, dispatch};
use db::create_pool;
use runtime::ComponentRuntime;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match env::var("RUNNER_MODE")
        .unwrap_or_else(|_| "monolith".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "monolith" => run_monolith().await,
        "gateway" => msa::gateway::run().await,
        "catalog" => msa::catalog::run().await,
        "order" => msa::order::run().await,
        mode => anyhow::bail!(
            "지원하지 않는 RUNNER_MODE입니다: {mode} (monolith, gateway, catalog, order 중 선택)"
        ),
    }
}

async fn run_monolith() -> Result<()> {
    let runtime = tokio::task::spawn_blocking(|| -> Result<ComponentRuntime> {
        let pool = create_pool()?;
        pool.get().context(
            "PostgreSQL에 연결할 수 없습니다. DATABASE_URL과 네트워크 경로를 확인하세요",
        )?;
        ComponentRuntime::load(pool)
    })
    .await
    .context("Host Runner 초기화 작업이 비정상 종료되었습니다")??;
    let runtime = Arc::new(runtime);
    let address: SocketAddr = env::var("APP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8091".to_string())
        .parse()
        .context("APP_ADDR 형식이 올바르지 않습니다")?;

    let app = Router::new()
        .fallback(any(dispatch))
        .with_state(AppState { runtime });
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Rust/Wasm commerce Host Runner started");

    axum::serve(listener, app).await?;
    Ok(())
}
