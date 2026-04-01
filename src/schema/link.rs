// Embedded SQL detection, ORM→table linking, schema edge production

use crate::schema::EdgeType;
use regex::Regex;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Task 12: Embedded SQL detection
// ---------------------------------------------------------------------------

pub struct EmbeddedSqlRef {
    pub table_name: String,
}

/// Detect SQL table references embedded in arbitrary source code.
///
/// Returns deduplicated references for strings that look like SQL (they
/// contain at least one DML/DDL keyword and at least one structural keyword).
/// Pure SQL files should be excluded by callers (they define tables, not
/// embed SQL as string literals in other languages).
pub fn detect_embedded_sql(content: &str) -> Vec<EmbeddedSqlRef> {
    // The pattern optionally skips IF [NOT] EXISTS between TABLE and the table name.
    let re = Regex::new(
        r"(?i)\b(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?([a-zA-Z_][a-zA-Z0-9_]*)",
    )
    .unwrap();

    let mut seen = HashSet::new();
    let mut refs = Vec::new();

    // Must contain at least one DML keyword AND one structural keyword
    let upper = content.to_uppercase();
    let has_dml = [
        "SELECT ", "INSERT ", "UPDATE ", "DELETE ", "CREATE ", "ALTER ", "DROP ",
    ]
    .iter()
    .any(|kw| upper.contains(kw));
    let has_structural = ["FROM ", "INTO ", "TABLE ", "SET ", "UPDATE ", "JOIN "]
        .iter()
        .any(|kw| upper.contains(kw));

    if !has_dml || !has_structural {
        return refs;
    }

    for cap in re.captures_iter(content) {
        let table = cap[1].to_string();
        // Filter out SQL reserved words that can appear after FROM/JOIN/INTO/UPDATE/TABLE
        let table_upper = table.to_uppercase();
        if [
            "SELECT",
            "WHERE",
            "AND",
            "OR",
            "SET",
            "VALUES",
            "INTO",
            "TABLE",
            "FROM",
            "JOIN",
            "ON",
            "AS",
            "NOT",
            "NULL",
            "IN",
            "EXISTS",
            "BETWEEN",
            "LIKE",
            "ORDER",
            "GROUP",
            "BY",
            "HAVING",
            "LIMIT",
            "OFFSET",
            "UNION",
            "ALL",
            "CASE",
            "WHEN",
            "THEN",
            "ELSE",
            "END",
            "TRUE",
            "FALSE",
            "IS",
            "CREATE",
            "ALTER",
            "DROP",
            "INSERT",
            "UPDATE",
            "DELETE",
            "IF",
            "ONLY",
            "LATERAL",
            "OUTER",
            "INNER",
            "LEFT",
            "RIGHT",
            "CROSS",
            "FULL",
            "NATURAL",
            "DISTINCT",
            "WITH",
            "RECURSIVE",
        ]
        .contains(&table_upper.as_str())
        {
            continue;
        }
        // Skip parameters ($1, $2, etc.) and variable markers
        if table.starts_with('$') || table.starts_with('?') || table.starts_with('@') {
            continue;
        }
        if seen.insert(table.clone()) {
            refs.push(EmbeddedSqlRef { table_name: table });
        }
    }
    refs
}

// ---------------------------------------------------------------------------
// Task 13: Build schema edges
// ---------------------------------------------------------------------------

/// Build typed edges that connect source files to schema artifacts.
///
/// Produces edges for:
/// - FK references between table definition files
/// - View → source table
/// - DB function → referenced table
/// - Embedded SQL in application code → table definition file
/// - ORM model file → table definition file
/// - Migration sequence (each migration → its predecessor)
pub fn build_schema_edges(
    files: &[crate::index::IndexedFile],
    schema_index: &crate::schema::SchemaIndex,
) -> Vec<(String, String, EdgeType)> {
    let mut edges = Vec::new();

    // FK edges: table file → referenced table file
    for table in schema_index.tables.values() {
        for col in &table.columns {
            if let Some(fk) = &col.foreign_key {
                if let Some(target) = schema_index.tables.get(&fk.target_table) {
                    if table.file_path != target.file_path {
                        edges.push((
                            table.file_path.clone(),
                            target.file_path.clone(),
                            EdgeType::ForeignKey,
                        ));
                    }
                }
            }
        }
    }

    // View → source table edges
    for view in schema_index.views.values() {
        for table_name in &view.source_tables {
            if let Some(table) = schema_index.tables.get(table_name) {
                edges.push((
                    view.file_path.clone(),
                    table.file_path.clone(),
                    EdgeType::ViewReference,
                ));
            }
        }
    }

    // Function → referenced table edges
    for func in schema_index.functions.values() {
        for table_name in &func.referenced_tables {
            if let Some(table) = schema_index.tables.get(table_name) {
                edges.push((
                    func.file_path.clone(),
                    table.file_path.clone(),
                    EdgeType::FunctionReference,
                ));
            }
        }
    }

    // Embedded SQL edges (scan all files)
    for file in files {
        let lang = file.language.as_deref().unwrap_or("");
        // Skip SQL files themselves (they define tables, not embed SQL)
        if lang == "sql" {
            continue;
        }

        let mut file_tables: HashSet<String> = HashSet::new();

        // Scan symbol bodies
        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                for sql_ref in detect_embedded_sql(&symbol.body) {
                    file_tables.insert(sql_ref.table_name);
                }
            }
        }

        // Scan file content directly (for module-level SQL, files without parse results)
        for sql_ref in detect_embedded_sql(&file.content) {
            file_tables.insert(sql_ref.table_name);
        }

        // Create edges for matched tables
        for table_name in &file_tables {
            if let Some(table) = schema_index.tables.get(table_name) {
                edges.push((
                    file.relative_path.clone(),
                    table.file_path.clone(),
                    EdgeType::EmbeddedSql,
                ));
            }
        }
    }

    // ORM model → table edges
    for model in schema_index.orm_models.values() {
        if let Some(table) = schema_index.tables.get(&model.table_name) {
            edges.push((
                model.file_path.clone(),
                table.file_path.clone(),
                EdgeType::OrmModel,
            ));
        }
    }

    // Migration sequence edges
    for chain in &schema_index.migrations {
        for i in 1..chain.migrations.len() {
            edges.push((
                chain.migrations[i].file_path.clone(),
                chain.migrations[i - 1].file_path.clone(),
                EdgeType::MigrationSequence,
            ));
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexedFile;
    use crate::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
    use crate::schema::{
        ColumnSchema, ForeignKeyRef, MigrationChain, MigrationEntry, MigrationFramework,
        OrmFramework, OrmModelSchema, SchemaIndex, TableSchema,
    };

    // -------------------------------------------------------------------------
    // Helper builders
    // -------------------------------------------------------------------------

    fn make_indexed_file(
        path: &str,
        language: Option<&str>,
        content: &str,
        symbols: Vec<Symbol>,
    ) -> IndexedFile {
        IndexedFile {
            relative_path: path.to_string(),
            language: language.map(|s| s.to_string()),
            size_bytes: content.len() as u64,
            token_count: 0,
            parse_result: Some(ParseResult {
                symbols,
                imports: vec![],
                exports: vec![],
            }),
            content: content.to_string(),
            mtime_secs: None,
        }
    }

    fn make_indexed_file_no_parse(
        path: &str,
        language: Option<&str>,
        content: &str,
    ) -> IndexedFile {
        IndexedFile {
            relative_path: path.to_string(),
            language: language.map(|s| s.to_string()),
            size_bytes: content.len() as u64,
            token_count: 0,
            parse_result: None,
            content: content.to_string(),
            mtime_secs: None,
        }
    }

    fn make_symbol_with_body(body: &str) -> Symbol {
        Symbol {
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            visibility: Visibility::Public,
            signature: "func test_func()".to_string(),
            body: body.to_string(),
            start_line: 1,
            end_line: 10,
        }
    }

    fn make_file_list(files: Vec<IndexedFile>) -> Vec<IndexedFile> {
        files
    }

    fn make_table(name: &str, file_path: &str, columns: Vec<ColumnSchema>) -> TableSchema {
        TableSchema {
            name: name.to_string(),
            columns,
            primary_key: None,
            indexes: vec![],
            file_path: file_path.to_string(),
            start_line: 1,
        }
    }

    fn make_column_with_fk(name: &str, target_table: &str, target_col: &str) -> ColumnSchema {
        ColumnSchema {
            name: name.to_string(),
            data_type: "INTEGER".to_string(),
            nullable: true,
            default: None,
            constraints: vec![],
            foreign_key: Some(ForeignKeyRef {
                target_table: target_table.to_string(),
                target_column: target_col.to_string(),
            }),
        }
    }

    fn make_column_plain(name: &str) -> ColumnSchema {
        ColumnSchema {
            name: name.to_string(),
            data_type: "TEXT".to_string(),
            nullable: true,
            default: None,
            constraints: vec![],
            foreign_key: None,
        }
    }

    // -------------------------------------------------------------------------
    // Task 12: detect_embedded_sql tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_detect_embedded_sql_select_from() {
        let content = r#"
            fn get_users(db: &DB) -> Vec<User> {
                db.query("SELECT id, name FROM users WHERE active = true")
            }
        "#;
        let refs = detect_embedded_sql(content);
        assert!(!refs.is_empty(), "should detect 'users' table reference");
        assert!(refs.iter().any(|r| r.table_name == "users"));
    }

    #[test]
    fn test_detect_embedded_sql_insert_into() {
        let content = r#"db.execute("INSERT INTO orders (user_id, total) VALUES ($1, $2)")"#;
        let refs = detect_embedded_sql(content);
        assert!(refs.iter().any(|r| r.table_name == "orders"));
    }

    #[test]
    fn test_detect_embedded_sql_update() {
        let content = r#"db.run("UPDATE products SET price = 10 WHERE id = 1")"#;
        let refs = detect_embedded_sql(content);
        assert!(refs.iter().any(|r| r.table_name == "products"));
    }

    #[test]
    fn test_detect_embedded_sql_join_multiple_tables() {
        let content = r#"
            let sql = "SELECT u.name, o.total FROM users u JOIN orders o ON u.id = o.user_id";
        "#;
        let refs = detect_embedded_sql(content);
        let names: Vec<&str> = refs.iter().map(|r| r.table_name.as_str()).collect();
        assert!(names.contains(&"users"), "should find 'users': {:?}", names);
        assert!(
            names.contains(&"orders"),
            "should find 'orders': {:?}",
            names
        );
    }

    #[test]
    fn test_detect_embedded_sql_not_sql_string() {
        // No DML keywords at all — should produce zero refs
        let content = r#"
            let message = "Hello, world!";
            let value = "some other string";
        "#;
        let refs = detect_embedded_sql(content);
        assert!(
            refs.is_empty(),
            "should not detect any SQL refs in non-SQL string"
        );
    }

    #[test]
    fn test_detect_embedded_sql_parameterized_queries() {
        // Parameters like $1, $2 should be filtered out
        let content = r#"db.query("SELECT * FROM accounts WHERE id = $1 AND status = $2")"#;
        let refs = detect_embedded_sql(content);
        assert!(refs.iter().any(|r| r.table_name == "accounts"));
        // Parameters should NOT appear as table refs
        assert!(!refs.iter().any(|r| r.table_name.starts_with('$')));
    }

    #[test]
    fn test_detect_embedded_sql_multiline() {
        let content = r#"
            let query = "
                SELECT u.id, p.name
                FROM users u
                JOIN profiles p ON u.id = p.user_id
                WHERE u.active = true
            ";
            db.execute(query);
        "#;
        let refs = detect_embedded_sql(content);
        let names: Vec<&str> = refs.iter().map(|r| r.table_name.as_str()).collect();
        assert!(names.contains(&"users"), "should find 'users': {:?}", names);
        assert!(
            names.contains(&"profiles"),
            "should find 'profiles': {:?}",
            names
        );
    }

    #[test]
    fn test_detect_embedded_sql_delete_from() {
        let content = r#"db.execute("DELETE FROM sessions WHERE expires_at < NOW()")"#;
        let refs = detect_embedded_sql(content);
        assert!(refs.iter().any(|r| r.table_name == "sessions"));
    }

    #[test]
    fn test_detect_embedded_sql_create_table_in_code() {
        // CREATE TABLE inside a migration helper function
        let content = r#"
            fn up(db: &DB) {
                db.execute("CREATE TABLE IF NOT EXISTS audit_logs (id SERIAL PRIMARY KEY)");
            }
        "#;
        let refs = detect_embedded_sql(content);
        assert!(
            refs.iter().any(|r| r.table_name == "audit_logs"),
            "should detect 'audit_logs': {:?}",
            refs.iter().map(|r| &r.table_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_detect_embedded_sql_empty_string() {
        let refs = detect_embedded_sql("");
        assert!(refs.is_empty());
    }

    // -------------------------------------------------------------------------
    // Task 13: build_schema_edges tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_schema_edges_fk() {
        // orders.user_id references users table
        let users_table = make_table("users", "schema/users.sql", vec![make_column_plain("id")]);
        let orders_table = make_table(
            "orders",
            "schema/orders.sql",
            vec![make_column_with_fk("user_id", "users", "id")],
        );

        let mut schema = SchemaIndex::empty();
        schema.tables.insert("users".to_string(), users_table);
        schema.tables.insert("orders".to_string(), orders_table);

        let files = make_file_list(vec![]);
        let edges = build_schema_edges(&files, &schema);

        assert!(
            edges
                .iter()
                .any(|(from, to, etype)| from == "schema/orders.sql"
                    && to == "schema/users.sql"
                    && *etype == EdgeType::ForeignKey),
            "should have FK edge from orders to users: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_embedded_sql() {
        // A Rust file embeds SQL that references the 'products' table
        let products_table = make_table("products", "schema/products.sql", vec![]);
        let mut schema = SchemaIndex::empty();
        schema.tables.insert("products".to_string(), products_table);

        let rust_file = make_indexed_file_no_parse(
            "src/repo.rs",
            Some("rust"),
            r#"
                pub fn list_products(db: &DB) -> Vec<Product> {
                    db.query("SELECT id, name FROM products WHERE active = true").await
                }
            "#,
        );

        let files = make_file_list(vec![rust_file]);
        let edges = build_schema_edges(&files, &schema);

        assert!(
            edges.iter().any(|(from, to, etype)| from == "src/repo.rs"
                && to == "schema/products.sql"
                && *etype == EdgeType::EmbeddedSql),
            "should have EmbeddedSql edge from repo.rs to products.sql: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_migration_sequence() {
        let chain = MigrationChain {
            framework: MigrationFramework::Rails,
            directory: "db/migrate".to_string(),
            migrations: vec![
                MigrationEntry {
                    file_path: "db/migrate/001_create_users.rb".to_string(),
                    sequence: "001".to_string(),
                    name: "create_users".to_string(),
                },
                MigrationEntry {
                    file_path: "db/migrate/002_add_email.rb".to_string(),
                    sequence: "002".to_string(),
                    name: "add_email".to_string(),
                },
                MigrationEntry {
                    file_path: "db/migrate/003_add_index.rb".to_string(),
                    sequence: "003".to_string(),
                    name: "add_index".to_string(),
                },
            ],
        };

        let mut schema = SchemaIndex::empty();
        schema.migrations.push(chain);

        let files = make_file_list(vec![]);
        let edges = build_schema_edges(&files, &schema);

        // 002 → 001, 003 → 002
        assert!(
            edges
                .iter()
                .any(|(from, to, etype)| from == "db/migrate/002_add_email.rb"
                    && to == "db/migrate/001_create_users.rb"
                    && *etype == EdgeType::MigrationSequence),
            "should have edge 002→001: {:?}",
            edges
        );
        assert!(
            edges
                .iter()
                .any(|(from, to, etype)| from == "db/migrate/003_add_index.rb"
                    && to == "db/migrate/002_add_email.rb"
                    && *etype == EdgeType::MigrationSequence),
            "should have edge 003→002: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_orm_to_table() {
        let users_table = make_table("users", "schema/users.sql", vec![]);
        let mut schema = SchemaIndex::empty();
        schema.tables.insert("users".to_string(), users_table);
        schema.orm_models.insert(
            "User".to_string(),
            OrmModelSchema {
                class_name: "User".to_string(),
                table_name: "users".to_string(),
                framework: OrmFramework::Django,
                file_path: "app/models.py".to_string(),
                fields: vec![],
            },
        );

        let files = make_file_list(vec![]);
        let edges = build_schema_edges(&files, &schema);

        assert!(
            edges.iter().any(|(from, to, etype)| from == "app/models.py"
                && to == "schema/users.sql"
                && *etype == EdgeType::OrmModel),
            "should have ORM→table edge: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_circular_fk() {
        // a.sql FK → b.sql, b.sql FK → a.sql (circular)
        let table_a = make_table(
            "table_a",
            "a.sql",
            vec![make_column_with_fk("b_id", "table_b", "id")],
        );
        let table_b = make_table(
            "table_b",
            "b.sql",
            vec![make_column_with_fk("a_id", "table_a", "id")],
        );

        let mut schema = SchemaIndex::empty();
        schema.tables.insert("table_a".to_string(), table_a);
        schema.tables.insert("table_b".to_string(), table_b);

        let files = make_file_list(vec![]);
        let edges = build_schema_edges(&files, &schema);

        // Both FK edges should appear
        assert!(
            edges.iter().any(|(from, to, etype)| from == "a.sql"
                && to == "b.sql"
                && *etype == EdgeType::ForeignKey),
            "should have FK a→b: {:?}",
            edges
        );
        assert!(
            edges.iter().any(|(from, to, etype)| from == "b.sql"
                && to == "a.sql"
                && *etype == EdgeType::ForeignKey),
            "should have FK b→a: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_sql_files_excluded_from_embedded_sql() {
        // A .sql file should NOT generate EmbeddedSql edges (it defines tables)
        let users_table = make_table("users", "schema.sql", vec![]);
        let mut schema = SchemaIndex::empty();
        schema.tables.insert("users".to_string(), users_table);

        // A SQL file referencing another table via SELECT
        let sql_file = make_indexed_file_no_parse(
            "other.sql",
            Some("sql"),
            "SELECT * FROM users WHERE id = 1;",
        );

        let files = make_file_list(vec![sql_file]);
        let edges = build_schema_edges(&files, &schema);

        // No EmbeddedSql edges should come from .sql files
        assert!(
            !edges
                .iter()
                .any(|(from, _, etype)| from == "other.sql" && *etype == EdgeType::EmbeddedSql),
            "SQL files should not generate EmbeddedSql edges: {:?}",
            edges
        );
    }

    #[test]
    fn test_schema_edges_symbol_body_scanned() {
        // Embedded SQL in a function body (via parse result symbols)
        let orders_table = make_table("orders", "db/schema.sql", vec![]);
        let mut schema = SchemaIndex::empty();
        schema.tables.insert("orders".to_string(), orders_table);

        let sym = make_symbol_with_body(r#"SELECT id, total FROM orders WHERE user_id = $1"#);
        let ts_file = make_indexed_file("src/orders.ts", Some("typescript"), "", vec![sym]);

        let files = make_file_list(vec![ts_file]);
        let edges = build_schema_edges(&files, &schema);

        assert!(
            edges.iter().any(|(from, to, etype)| from == "src/orders.ts"
                && to == "db/schema.sql"
                && *etype == EdgeType::EmbeddedSql),
            "should detect embedded SQL in symbol body: {:?}",
            edges
        );
    }
}
