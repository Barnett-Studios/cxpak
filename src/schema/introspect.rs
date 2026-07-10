//! Live database schema introspection (feature `data-introspect`).
//!
//! Connects **read-only** to a running Postgres or MySQL instance, reflects its
//! actual schema from the catalog, and maps the reflected rows into cxpak's
//! [`SchemaIndex`] vocabulary so the result can be diffed against the *code*
//! schema (the static index built from migrations / ORM / SQL).
//!
//! ## Binding constraints (ADR-0173)
//!
//! - **Credentials are never logged, persisted, or echoed.** The DSN is parsed
//!   once, used to open a connection, and dropped. Every error that could carry
//!   a connection string is wrapped in [`IntrospectError`], whose `Debug` and
//!   `Display` impls emit only a redacted, credential-free message. The raw DSN
//!   is never stored on the error and never reaches a log line.
//! - **Read-only.** Introspection issues `SELECT` / `SHOW`-style catalog queries
//!   only — no DDL, no DML. The Postgres path additionally sets the session to
//!   `default_transaction_read_only = on` as defense in depth.
//! - **Scoped async runtime.** Each entry point builds a dedicated
//!   single-threaded tokio runtime, drives the async work with `block_on`, and
//!   lets the runtime drop *after* `block_on` returns — never from inside an
//!   async context, and never nested inside another runtime.
//! - **No LLM.** This module is pure catalog reflection + deterministic mapping.
//! - **Deterministic output.** Reflected tables, columns, and keys are sorted so
//!   repeated runs against the same database produce byte-identical results.

use crate::core_graph::schema::{ColumnSchema, ForeignKeyRef, SchemaIndex, TableSchema};
use std::collections::BTreeMap;
use std::fmt;

/// Which dialect a DSN targets. Selected from the DSN scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    MySql,
}

impl Dialect {
    /// Infer the dialect from a DSN scheme without retaining the DSN.
    ///
    /// Only the scheme prefix is inspected; the credential-bearing remainder is
    /// never copied out of the borrowed input.
    pub fn from_dsn(dsn: &str) -> Result<Self, IntrospectError> {
        let scheme = dsn.split("://").next().unwrap_or("").to_ascii_lowercase();
        match scheme.as_str() {
            "postgres" | "postgresql" => Ok(Dialect::Postgres),
            "mysql" => Ok(Dialect::MySql),
            _ => Err(IntrospectError::UnsupportedScheme),
        }
    }
}

/// A credential-free error for every introspection failure path.
///
/// Driver errors frequently embed the connection string (host, user, and on
/// some failure modes the password). We never propagate those verbatim: each
/// variant carries only a fixed, non-secret description. The original driver
/// error is *summarized*, never stored, so a DSN can never leak through `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrospectError {
    /// DSN scheme is neither `postgres(ql)://` nor `mysql://`.
    UnsupportedScheme,
    /// The dedicated tokio runtime could not be constructed.
    RuntimeInit,
    /// Connecting / authenticating failed. Carries a redacted reason only.
    Connection(&'static str),
    /// A catalog query failed. Carries a redacted reason only.
    Query(&'static str),
}

impl fmt::Display for IntrospectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntrospectError::UnsupportedScheme => write!(
                f,
                "unsupported database DSN scheme (expected postgres:// or mysql://)"
            ),
            IntrospectError::RuntimeInit => {
                write!(f, "failed to initialize the introspection runtime")
            }
            IntrospectError::Connection(reason) => {
                write!(f, "database connection failed: {reason}")
            }
            IntrospectError::Query(reason) => {
                write!(f, "catalog query failed: {reason}")
            }
        }
    }
}

impl std::error::Error for IntrospectError {}

// ---------------------------------------------------------------------------
// Reflected catalog rows (driver-agnostic, pure data)
// ---------------------------------------------------------------------------

/// One reflected column row from a catalog query.
///
/// This is the driver-agnostic shape the live drivers produce and that the
/// pure mapping functions consume. Unit tests construct these directly with
/// **no live database**, exercising the full rows → typed-nodes path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedColumn {
    pub table: String,
    pub column: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub default: Option<String>,
    pub is_primary_key: bool,
}

/// One reflected foreign-key row from a catalog query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedForeignKey {
    pub table: String,
    pub column: String,
    pub target_table: String,
    pub target_column: String,
}

/// The raw reflected catalog: column rows + foreign-key rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReflectedSchema {
    pub columns: Vec<ReflectedColumn>,
    pub foreign_keys: Vec<ReflectedForeignKey>,
}

// ---------------------------------------------------------------------------
// Pure mapping: reflected rows -> SchemaIndex (no DB, fully unit-testable)
// ---------------------------------------------------------------------------

/// Map reflected catalog rows into a [`SchemaIndex`].
///
/// Deterministic: tables and their columns are emitted in sorted order, and the
/// primary-key list is sorted, so two runs over the same catalog produce
/// byte-identical output. `file_path` is set to `<live:{dialect}>` — a sentinel
/// marking these nodes as reflected-from-a-database rather than parsed from a
/// source file — and `start_line` is `0`.
pub fn map_reflected_to_index(reflected: &ReflectedSchema, dialect: Dialect) -> SchemaIndex {
    let origin = match dialect {
        Dialect::Postgres => "<live:postgres>",
        Dialect::MySql => "<live:mysql>",
    };

    // Group foreign keys by (table, column) for O(1) attachment to columns.
    let mut fk_by_col: BTreeMap<(String, String), ForeignKeyRef> = BTreeMap::new();
    for fk in &reflected.foreign_keys {
        fk_by_col.insert(
            (fk.table.clone(), fk.column.clone()),
            ForeignKeyRef {
                target_table: fk.target_table.clone(),
                target_column: fk.target_column.clone(),
            },
        );
    }

    // Group columns by table, preserving deterministic ordering via BTreeMap.
    let mut tables: BTreeMap<String, Vec<&ReflectedColumn>> = BTreeMap::new();
    for col in &reflected.columns {
        tables.entry(col.table.clone()).or_default().push(col);
    }

    let mut index = SchemaIndex::empty();

    for (table_name, mut cols) in tables {
        // Sort columns by name for deterministic output.
        cols.sort_by(|a, b| a.column.cmp(&b.column));

        let mut columns = Vec::with_capacity(cols.len());
        let mut primary_key: Vec<String> = Vec::new();

        for col in cols {
            if col.is_primary_key {
                primary_key.push(col.column.clone());
            }
            let foreign_key = fk_by_col
                .get(&(table_name.clone(), col.column.clone()))
                .cloned();
            columns.push(ColumnSchema {
                name: col.column.clone(),
                data_type: col.data_type.clone(),
                nullable: col.is_nullable,
                default: col.default.clone(),
                constraints: Vec::new(),
                foreign_key,
            });
        }

        primary_key.sort();
        let pk = if primary_key.is_empty() {
            None
        } else {
            Some(primary_key)
        };

        index.tables.insert(
            table_name.clone(),
            TableSchema {
                name: table_name,
                columns,
                primary_key: pk,
                indexes: Vec::new(),
                file_path: origin.to_string(),
                start_line: 0,
            },
        );
    }

    index
}

// ---------------------------------------------------------------------------
// Live connection (feature-gated drivers, scoped runtime, DSN-scrubbed)
// ---------------------------------------------------------------------------

/// Reflect the schema of a live database addressed by `dsn`.
///
/// Builds a dedicated, scoped tokio runtime; connects **read-only**; reflects
/// the catalog; and returns a mapped [`SchemaIndex`]. The `dsn` is borrowed,
/// used to connect, and never copied into the result or any error. On any
/// failure the returned [`IntrospectError`] contains no credentials.
///
/// # Runtime lifecycle
///
/// A fresh `current_thread` runtime is constructed here, `block_on` drives the
/// async connect+reflect to completion, then the runtime is dropped on this
/// (synchronous) stack frame *after* `block_on` returns. It is never dropped
/// from within an async task and is never nested inside an existing runtime —
/// callers must invoke this from a synchronous context.
///
/// Available only with the `data-introspect` feature (which pulls the rustls DB
/// drivers and a tokio runtime). The default build is DB-free.
#[cfg(feature = "data-introspect")]
pub fn introspect_live(dsn: &str) -> Result<SchemaIndex, IntrospectError> {
    let dialect = Dialect::from_dsn(dsn)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| IntrospectError::RuntimeInit)?;

    // `block_on` returns before `runtime` drops; the drop happens on this sync
    // frame, never inside an async context — no nested-runtime panic possible.
    let result = runtime.block_on(async {
        match dialect {
            Dialect::Postgres => reflect_postgres(dsn).await,
            Dialect::MySql => reflect_mysql(dsn).await,
        }
    });

    result.map(|reflected| map_reflected_to_index(&reflected, dialect))
}

#[cfg(feature = "data-introspect")]
async fn reflect_postgres(dsn: &str) -> Result<ReflectedSchema, IntrospectError> {
    use tokio_postgres_rustls::MakeRustlsConnect;

    // Ensure a rustls crypto provider is installed for this process. `ring` is
    // selected via the `data-introspect` feature's rustls features. Installing
    // the default provider is idempotent across repeated calls.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Empty root store: for local/dev introspection over a plain TCP socket the
    // TLS connector is never invoked. If the server *requires* TLS the handshake
    // fails closed (no roots to verify against) rather than trusting blindly —
    // the safe default. Operators terminating TLS supply roots out of band.
    let roots = rustls::RootCertStore::empty();
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tls = MakeRustlsConnect::new(tls_config);

    // `connect` parses the DSN. Any error here may embed the DSN, so we discard
    // the driver error and surface only a fixed, credential-free reason.
    let (client, connection) = tokio_postgres::connect(dsn, tls)
        .await
        .map_err(|_| IntrospectError::Connection("could not connect or authenticate"))?;

    // Drive the connection task; if it errors we ignore the (DSN-bearing) detail.
    let conn_handle = tokio::spawn(async move {
        let _ = connection.await;
    });

    // Defense in depth: pin the session read-only. Failure is non-fatal for
    // reflection but we treat it as a query error to stay strict.
    client
        .batch_execute("SET default_transaction_read_only = on")
        .await
        .map_err(|_| IntrospectError::Query("could not set read-only session"))?;

    let column_rows = client
        .query(PG_COLUMNS_SQL, &[])
        .await
        .map_err(|_| IntrospectError::Query("column reflection query failed"))?;

    // `try_get` (not the panicking `get`) so a non-standard catalog row shape
    // from a Postgres-wire-compatible engine surfaces as a clean, DSN-free
    // IntrospectError instead of unwinding the reflection task.
    const COL_SHAPE: IntrospectError =
        IntrospectError::Query("column reflection returned an unexpected row shape");
    let mut columns = Vec::with_capacity(column_rows.len());
    for row in &column_rows {
        let table: String = row.try_get(0).map_err(|_| COL_SHAPE)?;
        let column: String = row.try_get(1).map_err(|_| COL_SHAPE)?;
        let data_type: String = row.try_get(2).map_err(|_| COL_SHAPE)?;
        let is_nullable: String = row.try_get(3).map_err(|_| COL_SHAPE)?;
        let default: Option<String> = row.try_get(4).map_err(|_| COL_SHAPE)?;
        let is_pk: bool = row.try_get(5).map_err(|_| COL_SHAPE)?;
        columns.push(ReflectedColumn {
            table,
            column,
            data_type,
            is_nullable: is_nullable.eq_ignore_ascii_case("YES"),
            default,
            is_primary_key: is_pk,
        });
    }

    let fk_rows = client
        .query(PG_FOREIGN_KEYS_SQL, &[])
        .await
        .map_err(|_| IntrospectError::Query("foreign-key reflection query failed"))?;

    const FK_SHAPE: IntrospectError =
        IntrospectError::Query("foreign-key reflection returned an unexpected row shape");
    let mut foreign_keys = Vec::with_capacity(fk_rows.len());
    for row in &fk_rows {
        foreign_keys.push(ReflectedForeignKey {
            table: row.try_get(0).map_err(|_| FK_SHAPE)?,
            column: row.try_get(1).map_err(|_| FK_SHAPE)?,
            target_table: row.try_get(2).map_err(|_| FK_SHAPE)?,
            target_column: row.try_get(3).map_err(|_| FK_SHAPE)?,
        });
    }

    // Dropping the client closes the connection; abort the driver task so the
    // runtime can finish cleanly.
    drop(client);
    conn_handle.abort();

    Ok(ReflectedSchema {
        columns,
        foreign_keys,
    })
}

#[cfg(feature = "data-introspect")]
async fn reflect_mysql(dsn: &str) -> Result<ReflectedSchema, IntrospectError> {
    use mysql_async::prelude::Queryable;

    // mysql_async parses the DSN into Opts. A parse failure may echo the DSN, so
    // we map it to a fixed reason.
    let opts = mysql_async::Opts::from_url(dsn)
        .map_err(|_| IntrospectError::Connection("invalid connection options"))?;
    let pool = mysql_async::Pool::new(opts);
    let mut conn = pool
        .get_conn()
        .await
        .map_err(|_| IntrospectError::Connection("could not connect or authenticate"))?;

    // Defense in depth: read-only session.
    conn.query_drop("SET SESSION TRANSACTION READ ONLY")
        .await
        .map_err(|_| IntrospectError::Query("could not set read-only session"))?;

    let column_rows: Vec<(String, String, String, String, Option<String>)> = conn
        .query(MYSQL_COLUMNS_SQL)
        .await
        .map_err(|_| IntrospectError::Query("column reflection query failed"))?;

    let columns = column_rows
        .into_iter()
        .map(
            |(table, column, data_type, is_nullable, default)| ReflectedColumn {
                table,
                column,
                data_type,
                is_nullable: is_nullable.eq_ignore_ascii_case("YES"),
                default,
                is_primary_key: false,
            },
        )
        .collect::<Vec<_>>();

    // Primary keys are reflected separately and merged in.
    let pk_rows: Vec<(String, String)> = conn
        .query(MYSQL_PRIMARY_KEYS_SQL)
        .await
        .map_err(|_| IntrospectError::Query("primary-key reflection query failed"))?;

    let mut columns = columns;
    let pk_set: std::collections::HashSet<(String, String)> = pk_rows.into_iter().collect();
    for col in &mut columns {
        if pk_set.contains(&(col.table.clone(), col.column.clone())) {
            col.is_primary_key = true;
        }
    }

    let fk_rows: Vec<(String, String, String, String)> =
        conn.query(MYSQL_FOREIGN_KEYS_SQL)
            .await
            .map_err(|_| IntrospectError::Query("foreign-key reflection query failed"))?;

    let foreign_keys = fk_rows
        .into_iter()
        .map(
            |(table, column, target_table, target_column)| ReflectedForeignKey {
                table,
                column,
                target_table,
                target_column,
            },
        )
        .collect();

    drop(conn);
    // Disconnect the pool; ignore any (DSN-bearing) shutdown error.
    let _ = pool.disconnect().await;

    Ok(ReflectedSchema {
        columns,
        foreign_keys,
    })
}

// ---------------------------------------------------------------------------
// Catalog SQL (read-only SELECTs over information_schema / pg_catalog)
// ---------------------------------------------------------------------------

#[cfg(feature = "data-introspect")]
const PG_COLUMNS_SQL: &str = "\
SELECT c.table_name, c.column_name, c.data_type, c.is_nullable, c.column_default, \
       COALESCE(pk.is_pk, false) AS is_primary_key \
FROM information_schema.columns c \
LEFT JOIN ( \
  SELECT kcu.table_name, kcu.column_name, true AS is_pk \
  FROM information_schema.table_constraints tc \
  JOIN information_schema.key_column_usage kcu \
    ON tc.constraint_name = kcu.constraint_name \
   AND tc.table_schema = kcu.table_schema \
  WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = 'public' \
) pk ON pk.table_name = c.table_name AND pk.column_name = c.column_name \
WHERE c.table_schema = 'public' \
ORDER BY c.table_name, c.column_name";

#[cfg(feature = "data-introspect")]
const PG_FOREIGN_KEYS_SQL: &str = "\
SELECT kcu.table_name, kcu.column_name, ccu.table_name AS target_table, \
       ccu.column_name AS target_column \
FROM information_schema.table_constraints tc \
JOIN information_schema.key_column_usage kcu \
  ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
JOIN information_schema.constraint_column_usage ccu \
  ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema \
WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public' \
ORDER BY kcu.table_name, kcu.column_name";

#[cfg(feature = "data-introspect")]
const MYSQL_COLUMNS_SQL: &str = "\
SELECT TABLE_NAME, COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT \
FROM information_schema.COLUMNS \
WHERE TABLE_SCHEMA = DATABASE() \
ORDER BY TABLE_NAME, COLUMN_NAME";

#[cfg(feature = "data-introspect")]
const MYSQL_PRIMARY_KEYS_SQL: &str = "\
SELECT TABLE_NAME, COLUMN_NAME \
FROM information_schema.KEY_COLUMN_USAGE \
WHERE TABLE_SCHEMA = DATABASE() AND CONSTRAINT_NAME = 'PRIMARY' \
ORDER BY TABLE_NAME, COLUMN_NAME";

#[cfg(feature = "data-introspect")]
const MYSQL_FOREIGN_KEYS_SQL: &str = "\
SELECT TABLE_NAME, COLUMN_NAME, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
FROM information_schema.KEY_COLUMN_USAGE \
WHERE TABLE_SCHEMA = DATABASE() AND REFERENCED_TABLE_NAME IS NOT NULL \
ORDER BY TABLE_NAME, COLUMN_NAME";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pg_col(table: &str, column: &str, ty: &str, nullable: bool, pk: bool) -> ReflectedColumn {
        ReflectedColumn {
            table: table.into(),
            column: column.into(),
            data_type: ty.into(),
            is_nullable: nullable,
            default: None,
            is_primary_key: pk,
        }
    }

    #[test]
    fn dialect_from_postgres_dsn() {
        assert_eq!(
            Dialect::from_dsn("postgres://u:p@h/db").unwrap(),
            Dialect::Postgres
        );
        assert_eq!(
            Dialect::from_dsn("postgresql://u:p@h/db").unwrap(),
            Dialect::Postgres
        );
    }

    #[test]
    fn dialect_from_mysql_dsn() {
        assert_eq!(
            Dialect::from_dsn("mysql://u:p@h/db").unwrap(),
            Dialect::MySql
        );
    }

    #[test]
    fn dialect_rejects_unknown_scheme() {
        assert_eq!(
            Dialect::from_dsn("sqlite:///tmp/x.db").unwrap_err(),
            IntrospectError::UnsupportedScheme
        );
    }

    #[test]
    fn error_display_never_contains_credentials() {
        // Simulate the worst case: a connection error. The Display output must
        // be a fixed string with no host/user/password.
        let err = IntrospectError::Connection("could not connect or authenticate");
        let shown = format!("{err}");
        let debugged = format!("{err:?}");
        for needle in ["secretpass", "admin", "10.0.0.5", "://"] {
            assert!(!shown.contains(needle), "Display leaked: {shown}");
            assert!(!debugged.contains(needle), "Debug leaked: {debugged}");
        }
        assert!(shown.contains("database connection failed"));
    }

    #[test]
    fn map_synthetic_pg_rows_to_typed_nodes() {
        // Two tables; users has a composite-free PK on id; orders FKs to users.
        let reflected = ReflectedSchema {
            columns: vec![
                pg_col("users", "id", "integer", false, true),
                pg_col("users", "email", "text", false, false),
                pg_col("orders", "id", "integer", false, true),
                pg_col("orders", "user_id", "integer", false, false),
            ],
            foreign_keys: vec![ReflectedForeignKey {
                table: "orders".into(),
                column: "user_id".into(),
                target_table: "users".into(),
                target_column: "id".into(),
            }],
        };

        let index = map_reflected_to_index(&reflected, Dialect::Postgres);

        assert_eq!(index.tables.len(), 2);
        let users = index.tables.get("users").unwrap();
        assert_eq!(users.name, "users");
        // Columns sorted: email, id
        assert_eq!(users.columns[0].name, "email");
        assert_eq!(users.columns[1].name, "id");
        assert_eq!(users.primary_key, Some(vec!["id".to_string()]));
        assert_eq!(users.file_path, "<live:postgres>");

        let orders = index.tables.get("orders").unwrap();
        let user_id = orders.columns.iter().find(|c| c.name == "user_id").unwrap();
        let fk = user_id.foreign_key.as_ref().unwrap();
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn map_synthetic_mysql_rows_to_typed_nodes() {
        let reflected = ReflectedSchema {
            columns: vec![
                pg_col("product", "sku", "varchar", false, true),
                pg_col("product", "price", "decimal", true, false),
            ],
            foreign_keys: vec![],
        };
        let index = map_reflected_to_index(&reflected, Dialect::MySql);
        let product = index.tables.get("product").unwrap();
        assert_eq!(product.file_path, "<live:mysql>");
        assert_eq!(product.primary_key, Some(vec!["sku".to_string()]));
        let price = product.columns.iter().find(|c| c.name == "price").unwrap();
        assert!(price.nullable);
    }

    #[test]
    fn mapping_is_deterministic() {
        // Same input in a different order must yield an identical SchemaIndex.
        let a = ReflectedSchema {
            columns: vec![
                pg_col("t", "b", "int", false, false),
                pg_col("t", "a", "int", false, true),
            ],
            foreign_keys: vec![],
        };
        let b = ReflectedSchema {
            columns: vec![
                pg_col("t", "a", "int", false, true),
                pg_col("t", "b", "int", false, false),
            ],
            foreign_keys: vec![],
        };
        let ia = map_reflected_to_index(&a, Dialect::Postgres);
        let ib = map_reflected_to_index(&b, Dialect::Postgres);
        let ja = serde_json::to_string(&ia).unwrap();
        let jb = serde_json::to_string(&ib).unwrap();
        assert_eq!(ja, jb, "mapping must be order-independent / deterministic");
    }

    #[test]
    fn table_with_no_primary_key_has_none() {
        let reflected = ReflectedSchema {
            columns: vec![pg_col("log", "message", "text", true, false)],
            foreign_keys: vec![],
        };
        let index = map_reflected_to_index(&reflected, Dialect::Postgres);
        assert_eq!(index.tables.get("log").unwrap().primary_key, None);
    }

    #[cfg(feature = "data-introspect")]
    #[test]
    fn introspect_live_rejects_bad_scheme_without_leaking_dsn() {
        let err = introspect_live("redis://user:hunter2@example.com:6379/0").unwrap_err();
        assert_eq!(err, IntrospectError::UnsupportedScheme);
        let shown = format!("{err}");
        assert!(!shown.contains("hunter2"));
        assert!(!shown.contains("example.com"));
    }

    // ---- Gated live integration tests (require a running database) ----
    //
    // These are #[ignore]d so the default `cargo test` stays green offline.
    // Run with a live DB:
    //   CXPAK_PG_DSN=postgres://... cargo test --features data-introspect \
    //       -- --ignored introspect_live_postgres
    //   CXPAK_MYSQL_DSN=mysql://...  cargo test --features data-introspect \
    //       -- --ignored introspect_live_mysql
    // The DSN comes from the environment only — no secret literals in source.

    #[cfg(feature = "data-introspect")]
    #[test]
    #[ignore = "requires a live Postgres reachable via CXPAK_PG_DSN"]
    fn introspect_live_postgres() {
        let dsn = std::env::var("CXPAK_PG_DSN")
            .expect("set CXPAK_PG_DSN to a read-only Postgres connection string");
        let index = introspect_live(&dsn).expect("postgres reflection should succeed");
        // A reflected database should expose at least one table; deeper fixture
        // assertions are environment-specific and left to CI fixtures.
        assert!(
            !index.tables.is_empty(),
            "live Postgres reflected no tables"
        );
        for table in index.tables.values() {
            assert_eq!(table.file_path, "<live:postgres>");
        }
    }

    #[cfg(feature = "data-introspect")]
    #[test]
    #[ignore = "requires a live MySQL reachable via CXPAK_MYSQL_DSN"]
    fn introspect_live_mysql() {
        let dsn = std::env::var("CXPAK_MYSQL_DSN")
            .expect("set CXPAK_MYSQL_DSN to a read-only MySQL connection string");
        let index = introspect_live(&dsn).expect("mysql reflection should succeed");
        assert!(!index.tables.is_empty(), "live MySQL reflected no tables");
        for table in index.tables.values() {
            assert_eq!(table.file_path, "<live:mysql>");
        }
    }

    #[cfg(feature = "data-introspect")]
    #[test]
    #[ignore = "requires a live Postgres reachable via CXPAK_PG_DSN"]
    fn introspect_live_postgres_auth_failure_scrubs_dsn() {
        // Force an auth failure by appending a bogus password to the host.
        let base = std::env::var("CXPAK_PG_DSN")
            .expect("set CXPAK_PG_DSN to a read-only Postgres connection string");
        // Replace the credentials with an intentionally-wrong sentinel password.
        let bad = base.replacen("://", "://baduser:WRONGPASS_SENTINEL@", 1);
        let err = introspect_live(&bad).expect_err("auth must fail with bad credentials");
        let shown = format!("{err}");
        let debugged = format!("{err:?}");
        assert!(
            !shown.contains("WRONGPASS_SENTINEL") && !debugged.contains("WRONGPASS_SENTINEL"),
            "auth-failure error leaked credentials: {shown} / {debugged}"
        );
    }
}
