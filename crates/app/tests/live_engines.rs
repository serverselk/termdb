//! Integration tests against real database containers.
//!
//! Spin them up with `docker compose -f dev/compose.yaml up -d` and run
//! `cargo test --workspace`. Every field can be overridden via `TERMDB_TEST_*`
//! env vars; the defaults below match `dev/compose.yaml`. A test is skipped
//! (with a note) whenever the server cannot be reached, so machines without
//! the containers still get a green suite.
//!
//! Postgres: `TERMDB_TEST_PG_{HOST,PORT,USER,PASSWORD,DB}`  (defaults
//! `localhost`, `5433`, `pgtest`, `pgtest`, `shop`).
//!
//! MySQL: `TERMDB_TEST_MYSQL_{HOST,PORT,USER,PASSWORD}` (defaults
//! `localhost`, `3306`, `root`, `root`).

use termdb::db::engine::LiveSession;
use termdb_core::{ConnectionConfig, Engine};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn pg_config() -> (ConnectionConfig, String) {
    (
        ConnectionConfig {
            id: None,
            name: "pg-test".into(),
            engine: Engine::Postgres,
            host: env_or("TERMDB_TEST_PG_HOST", "localhost"),
            port: env_or("TERMDB_TEST_PG_PORT", "5433")
                .parse()
                .expect("valid port"),
            username: env_or("TERMDB_TEST_PG_USER", "pgtest"),
            database: Some(env_or("TERMDB_TEST_PG_DB", "shop")),
            ssl: false,
        },
        env_or("TERMDB_TEST_PG_PASSWORD", "pgtest"),
    )
}

fn mysql_config() -> (ConnectionConfig, String) {
    (
        ConnectionConfig {
            id: None,
            name: "mysql-test".into(),
            engine: Engine::Mysql,
            host: env_or("TERMDB_TEST_MYSQL_HOST", "localhost"),
            port: env_or("TERMDB_TEST_MYSQL_PORT", "3306")
                .parse()
                .expect("valid port"),
            username: env_or("TERMDB_TEST_MYSQL_USER", "root"),
            database: None,
            ssl: false,
        },
        env_or("TERMDB_TEST_MYSQL_PASSWORD", "root"),
    )
}

#[tokio::test]
async fn postgres_connects_and_lists_databases_and_tables() {
    let (cfg, password) = pg_config();
    let Ok(session) = LiveSession::connect(&cfg, &password).await else {
        eprintln!("skipping: cannot reach pg-test container");
        return;
    };

    assert!(
        !session.server_version.is_empty(),
        "server version reported"
    );
    let db = cfg.database.as_deref().expect("pg db set");
    assert!(
        session.databases.iter().any(|d| d == db),
        "{} listed among databases: {:?}",
        db,
        session.databases
    );

    let tables = session.tables(db).await.expect("list tables");
    assert!(!tables.is_empty(), "at least one table");
    for expected in ["customers", "orders", "products"] {
        assert!(
            tables.iter().any(|t| t == expected),
            "{expected} among {tables:?}"
        );
    }

    session.disconnect().await;
}

#[tokio::test]
async fn mysql_connects_and_lists_databases_and_tables() {
    let (cfg, password) = mysql_config();
    let Ok(session) = LiveSession::connect(&cfg, &password).await else {
        eprintln!("skipping: cannot reach mysql-test container");
        return;
    };

    assert!(
        !session.server_version.is_empty(),
        "server version reported"
    );
    assert!(!session.databases.is_empty(), "at least one database");

    // mysql_test is created by dev/compose.yaml.
    let tables = session.tables("mysql_test").await.expect("list tables");
    assert!(!tables.is_empty(), "at least one table in mysql_test");

    session.disconnect().await;
}

/// Guards the app's own runtime construction: `Backend::spawn` builds a
/// multi-thread runtime with only `enable_io`+`enable_time`, and sqlx needs
/// the IO driver. This mirrors that builder exactly instead of borrowing
/// `#[tokio::test]`'s (IO-enabled) runtime.
#[test]
fn app_style_runtime_connects_to_postgres() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("build app-style runtime");
    let (cfg, password) = pg_config();

    let session = rt.block_on(async { LiveSession::connect(&cfg, &password).await });
    let Ok(session) = session else {
        eprintln!("skipping: cannot reach pg-test container");
        return;
    };
    assert!(!session.databases.is_empty());
    rt.block_on(async { session.disconnect().await });
}
