use std::{env, str::FromStr};

use anyhow::{Context, Result, bail};
use native_tls::TlsConnector;
use postgres::Config;
use postgres_native_tls::MakeTlsConnector;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub type PgManager = PostgresConnectionManager<MakeTlsConnector>;
pub type DbPool = Pool<PgManager>;

pub struct HostState {
    pub pool: DbPool,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl HostState {
    pub fn new(pool: DbPool) -> Self {
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

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn parse_insecure_tls_setting(value: Option<&str>) -> Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };

    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" | "" => Ok(false),
        _ => bail!("PG_TLS_INSECURE는 true 또는 false로 설정해야 합니다"),
    }
}

pub fn create_pool() -> Result<DbPool> {
    create_pool_from_env("DATABASE_URL")
}

pub fn create_pool_from_env(variable: &str) -> Result<DbPool> {
    let database_url = env::var(variable).with_context(|| {
        format!("{variable} 환경변수가 필요합니다. 실제 접속정보는 .env에만 작성하세요")
    })?;
    let config =
        Config::from_str(&database_url).context("DATABASE_URL 형식이 올바르지 않습니다")?;

    let insecure_tls = parse_insecure_tls_setting(env::var("PG_TLS_INSECURE").ok().as_deref())?;
    let mut tls_builder = TlsConnector::builder();
    if insecure_tls {
        tracing::warn!(
            "PG_TLS_INSECURE=true: TLS 암호화는 유지하지만 인증서와 호스트명 검증을 생략합니다. 테스트 DB에서만 사용하세요"
        );
        tls_builder.danger_accept_invalid_certs(true);
        tls_builder.danger_accept_invalid_hostnames(true);
    }

    let tls = tls_builder
        .build()
        .map(MakeTlsConnector::new)
        .context("PostgreSQL TLS 설정에 실패했습니다")?;
    let manager = PostgresConnectionManager::new(config, tls);

    Pool::builder()
        .max_size(8)
        .build(manager)
        .with_context(|| format!("{variable} PostgreSQL 연결 풀을 만들지 못했습니다"))
}

#[cfg(test)]
mod tests {
    use super::parse_insecure_tls_setting;

    #[test]
    fn tls_verification_is_enabled_by_default() {
        assert!(!parse_insecure_tls_setting(None).unwrap());
        assert!(!parse_insecure_tls_setting(Some("false")).unwrap());
        assert!(!parse_insecure_tls_setting(Some("0")).unwrap());
    }

    #[test]
    fn insecure_tls_requires_an_explicit_true_value() {
        assert!(parse_insecure_tls_setting(Some("true")).unwrap());
        assert!(parse_insecure_tls_setting(Some("YES")).unwrap());
        assert!(parse_insecure_tls_setting(Some("1")).unwrap());
    }

    #[test]
    fn invalid_tls_setting_is_rejected() {
        assert!(parse_insecure_tls_setting(Some("enabled")).is_err());
    }
}
