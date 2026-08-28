use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use axum::{Json, http::StatusCode, response::IntoResponse};
use wasmtime::{Engine, component::Component};

use super::dto::ErrorBody;

pub fn address(default: &str) -> Result<SocketAddr> {
    env::var("APP_ADDR")
        .unwrap_or_else(|_| default.to_string())
        .parse()
        .context("APP_ADDR 형식이 올바르지 않습니다")
}

pub fn component(engine: &Engine, file: &str) -> Result<Component> {
    let directory = env::var_os("COMPONENT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/wasm32-wasip2/release")
        });
    let path = directory.join(file);
    Component::from_file(engine, &path).map_err(|error| {
        anyhow!(
            "Wasm Component를 불러오지 못했습니다 ({}): {error}",
            path.display()
        )
    })
}

#[derive(Debug)]
pub struct ServiceError {
    pub status: StatusCode,
    pub message: String,
}

impl ServiceError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "MSA service operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "서버 내부 처리에 실패했습니다.",
        )
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub async fn blocking<T>(
    operation: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ServiceError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(ServiceError::internal)?
        .map_err(ServiceError::internal)
}
