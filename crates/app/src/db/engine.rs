//! Live database connections (sqlx pools) and server discovery.
//!
//! M2 vertical slice: connect, list databases and tables. M3 adds column
//! metadata (describe) and paginated row reads. Segregated from egui so the
//! logic can be integration-tested against real containers without a UI.

use std::collections::HashMap;

use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgRow, PgSslMode};
use sqlx::{query, query_scalar, AssertSqlSafe, Column as ColumnInfo, Row, ValueRef};
use termdb_core::{ConnectionConfig, Engine};

pub const FILTER_OPS: &[&str] = &[
    "=", "!=", "<>", ">", "<", ">=", "<=", "LIKE", "NOT LIKE", "ILIKE",
];

fn valid_op(op: &str) -> bool {
    FILTER_OPS.contains(&op)
}

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
const PG_DESCRIBE: &str = r#"
    SELECT c.column_name,
           c.data_type,
           c.is_nullable,
           CASE WHEN EXISTS (
               SELECT 1 FROM information_schema.table_constraints tc
               JOIN information_schema.key_column_usage kcu
                 ON tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
               WHERE tc.constraint_type = 'PRIMARY KEY'
                 AND tc.table_schema = c.table_schema
                 AND tc.table_name = c.table_name
                 AND kcu.column_name = c.column_name
           ) THEN 1 ELSE 0 END,
           c.column_default,
           ''
    FROM information_schema.columns c
    WHERE c.table_schema = current_schema() AND c.table_name = $1
    ORDER BY c.ordinal_position"#;
const MYSQL_DESCRIBE: &str = r#"
    SELECT column_name, column_type, is_nullable,
           (column_key = 'PRI'), column_default, extra
    FROM information_schema.columns
    WHERE table_schema = ? AND table_name = ?
    ORDER BY ordinal_position"#;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no password stored for connection '{0}'")]
    MissingPassword(String),
    #[error("unsupported filter operator '{0}'")]
    InvalidOp(String),
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

/// A column, shaped like `Field / Type / Null / Key / Default / Extra` from
/// the Electron app's describe output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: String,
    pub nullable: bool,
    /// `"PRI"` for primary key columns, `""` otherwise (mysql naming kept).
    pub key: String,
    pub default: Option<String>,
    pub extra: String,
}

/// A typed WHERE filter, always bound as a parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableFilter {
    pub column: String,
    pub op: String,
    pub value: String,
}

/// A connected server plus everything discovered so far.
pub struct LiveSession {
    pub cfg: ConnectionConfig,
    pub server_version: String,
    pub databases: Vec<String>,
    /// MySQL: the single cross-database pool. Postgres: pool to the default
    /// database, used for server-level queries only.
    pool: LivePool,
    /// Postgres: lazy per-database pools, one per browsed database.
    pg_pools: HashMap<String, LivePool>,
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
                    pg_pools: HashMap::new(),
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
                    pg_pools: HashMap::new(),
                    base: PoolBase::Mysql,
                })
            }
        }
    }

    /// Pool usable for queries against `database`, opening one lazily for
    /// Postgres. MySQL shares its single pool across all databases.
    async fn db_pool(&mut self, database: &str) -> Result<&LivePool, EngineError> {
        match (&self.pool, &self.base) {
            (LivePool::Mysql(_), _) => Ok(&self.pool),
            (LivePool::Postgres(_), PoolBase::Postgres(base)) => {
                if !self.pg_pools.contains_key(database) {
                    let opts = base.clone().database(database);
                    let pool = PgPoolOptions::new()
                        .max_connections(2)
                        .connect_with(opts)
                        .await?;
                    self.pg_pools
                        .insert(database.to_owned(), LivePool::Postgres(pool));
                }
                Ok(self.pg_pools.get(database).expect("pg pool just opened"))
            }
            _ => unreachable!("pool and base always match their engine"),
        }
    }

    /// List base tables in `database`.
    pub async fn tables(&mut self, database: &str) -> Result<Vec<String>, EngineError> {
        let pool = self.db_pool(database).await?;
        match pool {
            LivePool::Mysql(pool) => Ok(query_scalar::<_, String>(MYSQL_TABLES)
                .bind(database)
                .fetch_all(pool)
                .await?),
            LivePool::Postgres(pool) => {
                Ok(query_scalar::<_, String>(PG_TABLES).fetch_all(pool).await?)
            }
        }
    }

    /// Describe a table's columns: Field/Type/Null/Key/Default/Extra.
    pub async fn describe(
        &mut self,
        database: &str,
        table: &str,
    ) -> Result<Vec<Column>, EngineError> {
        let pool = self.db_pool(database).await?;
        match pool {
            LivePool::Mysql(pool) => {
                let rows = query(MYSQL_DESCRIBE)
                    .bind(database)
                    .bind(table)
                    .fetch_all(pool)
                    .await?;
                Ok(rows.iter().map(mysql_column).collect())
            }
            LivePool::Postgres(pool) => {
                let rows = query(PG_DESCRIBE).bind(table).fetch_all(pool).await?;
                Ok(rows.iter().map(pg_column).collect())
            }
        }
    }

    /// Row count of a table, honoring an optional filter.
    pub async fn count(
        &mut self,
        database: &str,
        table: &str,
        columns: &[Column],
        filter: Option<&TableFilter>,
    ) -> Result<i64, EngineError> {
        let engine = self.cfg.engine;
        let pool = self.db_pool(database).await?;
        let where_sql = where_clause(engine, columns, filter)?.unwrap_or_default();
        let sql = format!(
            "SELECT COUNT(*) FROM {}{where_sql}",
            quote_ref(database, table, engine)
        );
        let filter_value = filter.map(|f| f.value.clone());
        match pool {
            LivePool::Mysql(pool) => {
                let mut q = query_scalar::<_, i64>(AssertSqlSafe(sql));
                if let Some(v) = filter_value {
                    q = q.bind(v);
                }
                q.fetch_one(pool).await.map_err(Into::into)
            }
            LivePool::Postgres(pool) => {
                let mut q = query_scalar::<_, i64>(AssertSqlSafe(sql));
                if let Some(v) = filter_value {
                    q = q.bind(v);
                }
                q.fetch_one(pool).await.map_err(Into::into)
            }
        }
    }

    /// One page of rows as text, `NULL` mapped to `None`. Columns are cast to
    /// text in SQL so a single row type covers every column type for display.
    pub async fn page(
        &mut self,
        database: &str,
        table: &str,
        columns: &[Column],
        filter: Option<&TableFilter>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Vec<Option<String>>>, EngineError> {
        let engine = self.cfg.engine;
        let pool = self.db_pool(database).await?;
        let select = columns
            .iter()
            .map(|c| cast_text(&c.name, engine))
            .collect::<Vec<_>>()
            .join(", ");
        let quoted_table = quote_ref(database, table, engine);
        let where_sql = where_clause(engine, columns, filter)?;
        let (limit_ph, offset_ph) = match (&pool, where_sql.is_some()) {
            (LivePool::Postgres(_), true) => ("$2", "$3"),
            (LivePool::Postgres(_), false) => ("$1", "$2"),
            (LivePool::Mysql(_), _) => ("?", "?"),
        };
        let sql = format!(
            "SELECT {select} FROM {quoted_table}{where} LIMIT {limit_ph} OFFSET {offset_ph}",
            where = where_sql.unwrap_or_default()
        );
        // Bind order matches placeholder order: filter value, then limit/offset.
        let mut binds: Vec<String> = Vec::new();
        if let Some(f) = filter {
            binds.push(f.value.clone());
        }
        match pool {
            LivePool::Mysql(pool) => {
                let mut q = query(AssertSqlSafe(sql));
                for b in binds {
                    q = q.bind(b);
                }
                let rows = q.bind(limit).bind(offset).fetch_all(pool).await?;
                Ok(rows.iter().map(mysql_row_to_text).collect())
            }
            LivePool::Postgres(pool) => {
                let mut q = query(AssertSqlSafe(sql));
                for b in binds {
                    q = q.bind(b);
                }
                let rows = q.bind(limit).bind(offset).fetch_all(pool).await?;
                Ok(rows.iter().map(pg_row_to_text).collect())
            }
        }
    }

    /// Run arbitrary SQL the user typed. Returns column names and rows as text
    /// (best-effort decode; undecodable non-null cells show `<binary>`).
    pub async fn query_results(
        &self,
        sql: &str,
    ) -> Result<(Vec<String>, Vec<Vec<Option<String>>>), EngineError> {
        match &self.pool {
            LivePool::Mysql(pool) => {
                let rows = query(AssertSqlSafe(sql.to_owned())).fetch_all(pool).await?;
                let columns = row_columns(&rows);
                Ok((columns, rows.iter().map(mysql_row_to_display).collect()))
            }
            LivePool::Postgres(pool) => {
                let rows = query(AssertSqlSafe(sql.to_owned())).fetch_all(pool).await?;
                let columns = row_columns(&rows);
                Ok((columns, rows.iter().map(pg_row_to_display).collect()))
            }
        }
    }

    /// Prepared-statement UPDATE by primary key. `values` covers the columns
    /// to set (the PK column is skipped) and `pk` is the current key value.
    pub async fn update_row(
        &mut self,
        database: &str,
        table: &str,
        columns: &[Column],
        values: &[(String, Option<String>)],
        pk: &(String, Option<String>),
    ) -> Result<u64, EngineError> {
        let engine = self.cfg.engine;
        let pool = self.db_pool(database).await?;
        let mut set_parts = Vec::new();
        let mut binds: Vec<Option<String>> = Vec::new();
        for (name, value) in values {
            if name == &pk.0 {
                continue;
            }
            let ty = col_type(columns, name);
            let ph = placeholder(engine, binds.len(), ty);
            set_parts.push(format!("{} = {ph}", quote_table(name, engine)));
            binds.push(value.clone());
        }
        let pk_ty = col_type(columns, &pk.0);
        let where_ph = placeholder(engine, binds.len(), pk_ty);
        binds.push(pk.1.clone());
        let rows = execute_parametrized(
            pool,
            &format!(
                "UPDATE {} SET {} WHERE {} = {where_ph}",
                quote_ref(database, table, engine),
                set_parts.join(", "),
                quote_table(&pk.0, engine)
            ),
            binds,
        )
        .await?;
        Ok(rows)
    }

    /// Prepared-statement DELETE by primary key.
    pub async fn delete_row(
        &mut self,
        database: &str,
        table: &str,
        columns: &[Column],
        pk: &(String, Option<String>),
    ) -> Result<u64, EngineError> {
        let engine = self.cfg.engine;
        let pool = self.db_pool(database).await?;
        let pk_ty = col_type(columns, &pk.0);
        let ph = placeholder(engine, 0, pk_ty);
        let sql = format!(
            "DELETE FROM {} WHERE {} = {ph}",
            quote_ref(database, table, engine),
            quote_table(&pk.0, engine)
        );
        execute_parametrized(pool, &sql, vec![pk.1.clone()]).await
    }

    /// Prepared-statement INSERT for every described column; `None` values
    /// become SQL NULL (autoincrement/serial PKs will still assign).
    pub async fn insert_row(
        &mut self,
        database: &str,
        table: &str,
        columns: &[Column],
        values: &[(String, Option<String>)],
    ) -> Result<u64, EngineError> {
        let engine = self.cfg.engine;
        let pool = self.db_pool(database).await?;
        let mut names = Vec::new();
        let mut placeholders = Vec::new();
        let mut binds: Vec<Option<String>> = Vec::new();
        for (i, (name, value)) in values.iter().enumerate() {
            // Let the database apply default on serial/auto-increment PKs and
            // other defaulted columns: omit them when no value is supplied.
            let has_default = columns.get(i).map(|c| c.default.is_some()).unwrap_or(false);
            if value.is_none() && has_default {
                continue;
            }
            names.push(quote_table(name, engine));
            placeholders.push(placeholder(engine, binds.len(), col_type(columns, name)));
            binds.push(value.clone());
        }
        execute_parametrized(
            pool,
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                quote_ref(database, table, engine),
                names.join(", "),
                placeholders.join(", ")
            ),
            binds,
        )
        .await
    }

    /// Close the pools and release the file descriptors.
    pub async fn disconnect(self) {
        for pool in self.pg_pools.values() {
            if let LivePool::Postgres(pool) = pool {
                pool.close().await;
            }
        }
        match self.pool {
            LivePool::Postgres(pool) => pool.close().await,
            LivePool::Mysql(pool) => pool.close().await,
        }
    }
}

/// Run a parametrized statement with the given binds on the engine's pool.
async fn execute_parametrized(
    pool: &LivePool,
    sql: &str,
    binds: Vec<Option<String>>,
) -> Result<u64, EngineError> {
    match pool {
        LivePool::Mysql(pool) => {
            let mut q = query(AssertSqlSafe(sql.to_owned()));
            for b in binds {
                q = q.bind(b);
            }
            Ok(q.execute(pool).await?.rows_affected())
        }
        LivePool::Postgres(pool) => {
            let mut q = query(AssertSqlSafe(sql.to_owned()));
            for b in binds {
                q = q.bind(b);
            }
            Ok(q.execute(pool).await?.rows_affected())
        }
    }
}

/// Convert one row to display strings; `NULL` becomes `None`.
fn mysql_row_to_text(row: &MySqlRow) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| row.try_get::<Option<String>, _>(i).unwrap_or(None))
        .collect()
}

fn pg_row_to_text(row: &PgRow) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| row.try_get::<Option<String>, _>(i).unwrap_or(None))
        .collect()
}

/// Best-effort text decode for arbitrary query results. Tries the common
/// scalar types in order so the grid can display ad-hoc SELECTs.
fn mysql_cell(row: &MySqlRow, index: usize) -> Option<String> {
    if let Ok(v) = row.try_get::<Option<String>, _>(index) {
        return v;
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(index) {
        return v.map(|x| x.to_string());
    }
    match row.try_get_raw(index) {
        Ok(raw) if !raw.is_null() => Some("<binary>".into()),
        _ => None,
    }
}

fn pg_cell(row: &PgRow, index: usize) -> Option<String> {
    if let Ok(v) = row.try_get::<Option<String>, _>(index) {
        return v;
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<f32>, _>(index) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(index) {
        return v.map(|x| x.to_string());
    }
    match row.try_get_raw(index) {
        Ok(raw) if !raw.is_null() => Some("<binary>".into()),
        _ => None,
    }
}

/// Query results recycle the row-to-text path per cell so ad-hoc queries and
/// browsed tables share one grid renderer.
fn mysql_row_to_display(row: &MySqlRow) -> Vec<Option<String>> {
    (0..row.len()).map(|i| mysql_cell(row, i)).collect()
}

fn pg_row_to_display(row: &PgRow) -> Vec<Option<String>> {
    (0..row.len()).map(|i| pg_cell(row, i)).collect()
}

fn row_columns<R: Row>(rows: &[R]) -> Vec<String> {
    rows.first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|col| col.name().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn mysql_column(row: &sqlx::mysql::MySqlRow) -> Column {
    Column {
        name: row.get(0),
        ty: row.get(1),
        nullable: row.get::<String, _>(2) == "YES",
        key: if row.get::<i64, _>(3) != 0 {
            "PRI".into()
        } else {
            String::new()
        },
        default: row.get(4),
        extra: row.get(5),
    }
}

fn pg_column(row: &sqlx::postgres::PgRow) -> Column {
    Column {
        name: row.get(0),
        ty: row.get(1),
        nullable: row.get::<String, _>(2) == "YES",
        key: if row.get::<i32, _>(3) != 0 {
            "PRI".into()
        } else {
            String::new()
        },
        default: row.get(4),
        extra: row.get(5),
    }
}

/// Quote a table name as an identifier for the engine.
fn quote_table(table: &str, engine: Engine) -> String {
    match engine {
        Engine::Postgres => format!("\"{}\"", table.replace('"', "\"\"")),
        Engine::Mysql => format!("`{}`", table.replace('`', "``")),
    }
}

/// Fully-qualified, quoted table reference. MySQL pools may have no default
/// database, so they need the `db`.`table` qualified form.
fn quote_ref(database: &str, table: &str, engine: Engine) -> String {
    match engine {
        Engine::Postgres => quote_table(table, engine),
        Engine::Mysql => format!(
            "`{}`.`{}`",
            database.replace('`', "``"),
            table.replace('`', "``")
        ),
    }
}

/// Render a column as a text-typed expression for display.
fn cast_text(column: &str, engine: Engine) -> String {
    match engine {
        Engine::Postgres => format!("\"{}\"::text", column.replace('"', "\"\"")),
        Engine::Mysql => format!("CAST(`{}` AS CHAR)", column.replace('`', "``")),
    }
}

/// True when the `WHERE "col"` fragment (with a bound parameter) is desired.
/// The operator comes from a fixed whitelist so it can never be injected.
fn where_clause(
    engine: Engine,
    columns: &[Column],
    filter: Option<&TableFilter>,
) -> Result<Option<String>, EngineError> {
    let Some(f) = filter else {
        return Ok(None);
    };
    if !valid_op(&f.op) {
        return Err(EngineError::InvalidOp(f.op.clone()));
    }
    let ty = col_type(columns, &f.column);
    let ph = placeholder(engine, 0, ty);
    Ok(Some(format!(
        " WHERE {} {op} {ph}",
        quote_table(&f.column, engine),
        op = f.op
    )))
}

fn col_type<'a>(columns: &'a [Column], name: &str) -> &'a str {
    columns
        .iter()
        .find(|c| c.name == name)
        .map(|c| c.ty.as_str())
        .unwrap_or("text")
}

/// Positional parameter for the engine. Postgres needs a CAST to the column
/// type so bound text values coerce correctly; MySQL coerces automatically.
fn placeholder(engine: Engine, index: usize, ty: &str) -> String {
    match engine {
        Engine::Postgres => format!("CAST(${} AS {ty})", index + 1),
        Engine::Mysql => "?".to_owned(),
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
