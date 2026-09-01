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

use termdb::db::engine::{LiveSession, TableFilter};
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
    let Ok(mut session) = LiveSession::connect(&cfg, &password).await else {
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
    let Ok(mut session) = LiveSession::connect(&cfg, &password).await else {
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

#[tokio::test]
async fn postgres_describes_and_paginates() {
    let (cfg, password) = pg_config();
    let mut session = match LiveSession::connect(&cfg, &password).await {
        Ok(session) => session,
        Err(_) => {
            eprintln!("skipping: cannot reach pg-test container");
            return;
        }
    };
    let db = cfg.database.clone().expect("pg db set");

    let columns = session.describe(&db, "customers").await.expect("describe");
    assert!(!columns.is_empty());
    let id = &columns[0];
    assert_eq!(id.name, "id");
    assert_eq!(id.key, "PRI");
    assert!(!id.nullable);
    assert!(columns.iter().any(|c| c.name == "notes" && c.nullable));
    assert!(columns.iter().all(|c| !c.ty.is_empty()), "types reported");

    let total = session
        .count(&db, "customers", &columns, None)
        .await
        .expect("count");
    assert!(total >= 25, "enough rows to paginate");

    let page_a = session
        .page(&db, "customers", &columns, None, 10, 0)
        .await
        .expect("page a");
    let page_b = session
        .page(&db, "customers", &columns, None, 10, 10)
        .await
        .expect("page b");
    assert_eq!(page_a.len(), 10);
    assert_eq!(page_b.len(), 10);
    assert!(page_a.iter().all(|r| r.len() == columns.len()));
    let ids: Vec<i64> = page_a
        .iter()
        .map(|r| r[0].as_deref().unwrap().parse().unwrap())
        .collect();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "rows come back in primary-key order (stable across updates)"
    );
    let id_a: i64 = page_a[0][0].as_deref().unwrap().parse().unwrap();
    let id_b: i64 = page_b[0][0].as_deref().unwrap().parse().unwrap();
    assert!(id_b > id_a, "offset page starts further along");
    assert!(
        page_a.iter().any(|r| r.iter().any(Option::is_none)),
        "some NULL cells render as None"
    );

    session.disconnect().await;
}

#[tokio::test]
async fn mysql_describes_and_paginates() {
    let (cfg, password) = mysql_config();
    let mut session = match LiveSession::connect(&cfg, &password).await {
        Ok(session) => session,
        Err(_) => {
            eprintln!("skipping: cannot reach mysql-test container");
            return;
        }
    };

    let columns = session
        .describe("mysql_test", "customers")
        .await
        .expect("describe");
    assert_eq!(columns[0].name, "id");
    assert_eq!(columns[0].key, "PRI");

    let total = session
        .count("mysql_test", "customers", &columns, None)
        .await
        .expect("count");
    assert!(total >= 1);

    let page = session
        .page("mysql_test", "customers", &columns, None, 5, 0)
        .await
        .expect("page");
    assert_eq!(page.len() as i64, total);
    assert!(page[0][0].as_deref().is_some());

    session.disconnect().await;
}

#[tokio::test]
async fn postgres_query_filter_and_row_mutations() {
    let (cfg, password) = pg_config();
    let mut session = match LiveSession::connect(&cfg, &password).await {
        Ok(session) => session,
        Err(_) => {
            eprintln!("skipping: cannot reach pg-test container");
            return;
        }
    };
    let db = cfg.database.clone().expect("pg db set");
    let columns = session.describe(&db, "customers").await.expect("describe");
    let rows_before = session
        .count(&db, "customers", &columns, None)
        .await
        .unwrap();

    // Ad-hoc query on the default pool.
    let sql = "SELECT id, name FROM customers WHERE id <= 3 ORDER BY id";
    let (cols, rows) = session.query_results(sql).await.expect("run query");
    assert_eq!(cols, vec!["id".to_string(), "name".to_string()]);
    assert_eq!(rows.len(), 3);

    // WHERE builder filter equality (is_active = true).
    let filter = TableFilter {
        column: "is_active".into(),
        op: "=".into(),
        value: "true".into(),
    };
    let filtered_total = session
        .count(&db, "customers", &columns, Some(&filter))
        .await
        .expect("filtered count");
    assert!(filtered_total > 0 && filtered_total <= rows_before);
    let filtered_page = session
        .page(&db, "customers", &columns, Some(&filter), 10, 0)
        .await
        .expect("filtered page");
    assert!(!filtered_page.is_empty());
    let is_active_idx = columns.iter().position(|c| c.name == "is_active").unwrap();
    assert!(
        filtered_page
            .iter()
            .all(|r| r[is_active_idx].as_deref() == Some("true")),
        "filtered rows all is_active"
    );

    // Insert a row, find it, update it, delete it.
    let probe = format!(
        "m4test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let insert_values: Vec<(String, Option<String>)> = columns
        .iter()
        .map(|c| match c.name.as_str() {
            "name" => (c.name.clone(), Some(probe.clone())),
            "email" => (c.name.clone(), Some(format!("{probe}@example.com"))),
            _ => (c.name.clone(), None),
        })
        .collect();
    session
        .insert_row(&db, "customers", &columns, &insert_values)
        .await
        .expect("insert");
    assert_eq!(
        session
            .count(&db, "customers", &columns, None)
            .await
            .unwrap(),
        rows_before + 1,
        "row inserted"
    );

    let (_, probe_rows) = session
        .query_results(&format!("SELECT id FROM customers WHERE name = '{probe}'"))
        .await
        .expect("find probe");
    let probe_id = probe_rows[0][0].clone().expect("probe id");

    // Record the probe's position in id order before updating it.
    let (_, id_rows) = session
        .query_results("SELECT id FROM customers ORDER BY id")
        .await
        .expect("id list");
    let pos_before = id_rows
        .iter()
        .position(|r| r[0].as_deref() == Some(probe_id.as_str()))
        .expect("probe present");

    // UPDATE city by primary key.
    let update_values: Vec<(String, Option<String>)> = columns
        .iter()
        .map(|c| match c.name.as_str() {
            "name" => (c.name.clone(), Some(probe.clone())),
            "email" => (c.name.clone(), Some(format!("{probe}@example.com"))),
            "city" => (c.name.clone(), Some("Braga".into())),
            _ => (c.name.clone(), None),
        })
        .collect();
    session
        .update_row(
            &db,
            "customers",
            &columns,
            &update_values,
            &("id".to_owned(), Some(probe_id.clone())),
        )
        .await
        .expect("update");
    {
        let (_, city_rows) = session
            .query_results(&format!("SELECT city FROM customers WHERE id = {probe_id}"))
            .await
            .expect("re-read");
        assert_eq!(city_rows[0][0].as_deref(), Some("Braga"));
    }

    // The updated row must keep its spot: the update rewrites the tuple at
    // the end of the heap, but id-ordered queries must not move it.
    let (_, id_rows) = session
        .query_results("SELECT id FROM customers ORDER BY id")
        .await
        .expect("id list after update");
    let pos_after = id_rows
        .iter()
        .position(|r| r[0].as_deref() == Some(probe_id.as_str()))
        .expect("probe still present");
    assert_eq!(pos_after, pos_before, "updated row keeps its position");

    // DELETE by primary key.
    session
        .delete_row(
            &db,
            "customers",
            &columns,
            &("id".to_owned(), Some(probe_id)),
        )
        .await
        .expect("delete");
    assert_eq!(
        session
            .count(&db, "customers", &columns, None)
            .await
            .unwrap(),
        rows_before,
        "row deleted"
    );

    session.disconnect().await;
}
