//! Live database connections (sqlx pools) and server discovery.
//!
//! M2 vertical slice: connect to a MySQL or Postgres server, list databases,
//! and list tables inside a database. Segregated from egui so the logic can
//! be integration-tested against real containers without a UI.

use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode};
use sqlx::query_scalar;
use termdb_core::{ConnectionConfig, Engine};

const PG_DATABASES: &str =
    "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname";
const MYSQL_DATABASES: &str = "SHOW DATABASES";
const PG_TABLES: &str = r#"
    SELECT table_name FROM information_schema.tables
    WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
      AND table_type = 'BASE TABLE'
    ORDER BY table_name"#;
const MYSQL_TABLES: &str = r#"
    SELECT table_name FROM information_schema.tables
    WHERE table_schema = ? AND table_type = 'BASE TABLE'
    ORDER BY table_name"#;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no password stored for connection '{0}'")]
    MissingPassword(String),
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Target options (host/port/credentials) needed to open per-database pools
/// for Postgres, whose connections are bound to a single database. MySQL can
/// answer across databases from one pool, so it needs no stored base.
#[derive(Debug, Clone)]
enum PoolBase {
    Postgres(Box<PgConnectOptions>),
    Mysql,
}

enum LivePool {
    Postgres(PgPool),
    Mysql(MySqlPool),
}

/// A connected server plus everything discovered so far.
pub struct LiveSession {
    pub cfg: ConnectionConfig,
    pub server_version: String,
    pub databases: Vec<String>,
    pool: LivePool,
    base: PoolBase,
}

impl LiveSession {
    /// Open a pool to `cfg`, sanity-check it and list databases.
    pub async fn connect(cfg: &ConnectionConfig, password: &str) -> Result<Self, EngineError> {
        match cfg.engine {
            Engine::Postgres => {
                let opts = base_pg_options(cfg, password);
                let pool = PgPoolOptions::new()
                    .max_connections(4)
                    .connect_with(opts.clone())
                    .await?;
                let server_version = query_scalar::<_, String>("SELECT version()")
                    .fetch_one(&pool)
                    .await?;
                let databases = query_scalar::<_, String>(PG_DATABASES)
                    .fetch_all(&pool)
                    .await?;
                Ok(Self {
                    cfg: cfg.clone(),
                    server_version,
                    databases,
                    pool: LivePool::Postgres(pool),
                    base: PoolBase::Postgres(Box::new(opts)),
                })
            }
            Engine::Mysql => {
                let opts = base_mysql_options(cfg, password);
                let pool = MySqlPoolOptions::new()
                    .max_connections(4)
                    .connect_with(opts.clone())
                    .await?;
                let server_version = query_scalar::<_, String>("SELECT VERSION()")
                    .fetch_one(&pool)
                    .await?;
                let databases = query_scalar::<_, String>(MYSQL_DATABASES)
                    .fetch_all(&pool)
                    .await?;
                Ok(Self {
                    cfg: cfg.clone(),
                    server_version,
                    databases,
                    pool: LivePool::Mysql(pool),
                    base: PoolBase::Mysql,
                })
            }
        }
    }

    /// List base tables in `database`.
    ///
    /// MySQL can answer from `information_schema` through the existing pool;
    /// Postgres needs a connection bound to the target database.
    pub async fn tables(&self, database: &str) -> Result<Vec<String>, EngineError> {
        match (&self.pool, &self.base) {
            (LivePool::Mysql(pool), PoolBase::Mysql) => Ok(query_scalar::<_, String>(MYSQL_TABLES)
                .bind(database)
                .fetch_all(pool)
                .await?),
            (LivePool::Postgres(_), PoolBase::Postgres(base)) => {
                let opts = base.clone().database(database);
                let pool = PgPoolOptions::new()
                    .max_connections(2)
                    .connect_with(opts)
                    .await?;
                let tables = query_scalar::<_, String>(PG_TABLES)
                    .fetch_all(&pool)
                    .await?;
                pool.close().await;
                Ok(tables)
            }
            _ => unreachable!("pool and base always match their engine"),
        }
    }

    /// Close the pool and release the file descriptors.
    pub async fn disconnect(self) {
        match self.pool {
            LivePool::Postgres(pool) => pool.close().await,
            LivePool::Mysql(pool) => pool.close().await,
        }
    }
}

fn base_pg_options(cfg: &ConnectionConfig, password: &str) -> PgConnectOptions {
    let database = cfg
        .database
        .clone()
        .unwrap_or_else(|| "postgres".to_owned());
    let mut opts = PgConnectOptions::new()
        .host(&cfg.host)
        .port(cfg.port)
        .username(&cfg.username)
        .password(password)
        .database(&database);
    if !cfg.ssl {
        opts = opts.ssl_mode(PgSslMode::Disable);
    }
    opts
}

fn base_mysql_options(cfg: &ConnectionConfig, password: &str) -> MySqlConnectOptions {
    let mut opts = MySqlConnectOptions::new()
        .host(&cfg.host)
        .port(cfg.port)
        .username(&cfg.username)
        .password(password);
    if let Some(database) = &cfg.database {
        opts = opts.database(database);
    }
    if !cfg.ssl {
        opts = opts.ssl_mode(MySqlSslMode::Disabled);
    }
    opts
}
