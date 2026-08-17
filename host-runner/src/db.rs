use std::{env, str::FromStr};

use anyhow::{Context, Result};
use native_tls::TlsConnector;
use postgres::Config;
use postgres_native_tls::MakeTlsConnector;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;

pub type PgManager = PostgresConnectionManager<MakeTlsConnector>;
pub type DbPool = Pool<PgManager>;

#[derive(Clone)]
pub struct HostState {
    pub pool: DbPool,
}

pub fn create_pool() -> Result<DbPool> {
    let database_url = env::var("DATABASE_URL")
        .context("DATABASE_URL 환경변수가 필요합니다. 실제 접속정보는 .env에만 작성하세요")?;
    let config =
        Config::from_str(&database_url).context("DATABASE_URL 형식이 올바르지 않습니다")?;
    let tls = TlsConnector::builder()
        .build()
        .map(MakeTlsConnector::new)
        .context("PostgreSQL TLS 설정에 실패했습니다")?;
    let manager = PostgresConnectionManager::new(config, tls);

    Pool::builder()
        .max_size(8)
        .build(manager)
        .context("PostgreSQL 연결 풀을 만들지 못했습니다")
}
