// Embedded SQL detection, ORM→table linking, schema edge production

use crate::schema::{EdgeConfidence, EdgeType, SchemaIndex};
use regex::Regex;
use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;

/// Matches a `SELECT ... FROM` projection list. Capture group 1 is the raw
/// column list between `SELECT` and `FROM`.
static RE_SELECT_LIST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)\bSELECT\s+(.+?)\s+FROM\b").expect("RE_SELECT_LIST"));

/// Matches a `column = value` / `column <op> value` comparison or assignment in
/// WHERE / SET / ON clauses. Capture group 1 is the (optionally table-qualified)
/// column reference, e.g. `email` or `u.email`.
static RE_COL_PREDICATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-z_][a-z0-9_]*(?:\.[a-z_][a-z0-9_]*)?)\s*(?:=|<>|!=|<=|>=|<|>|\bLIKE\b|\bIN\b|\bIS\b)")
        .expect("RE_COL_PREDICATE")
});

/// Matches an `INSERT INTO table (col, col, ...)` column list. Capture group 1
/// is the raw parenthesized column list.
static RE_INSERT_COLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\bINSERT\s+INTO\s+[a-z_][a-z0-9_]*\s*\(([^)]*)\)").expect("RE_INSERT_COLS")
});

static RE_EMBEDDED_SQL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:FROM|JOIN|INTO|UPDATE|TABLE)\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?([a-zA-Z_][a-zA-Z0-9_]*)",
    )
    .expect("RE_EMBEDDED_SQL")
});

// ---------------------------------------------------------------------------
// Task 12: Embedded SQL detection
// ---------------------------------------------------------------------------

#[derive(Debug)]
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
    let re = &*RE_EMBEDDED_SQL;

    let mut seen = HashSet::new();
    let mut refs = Vec::new();

    // Must contain at least one DML keyword AND one structural keyword.
    // UPDATE is intentionally excluded from the DML guard: it is too often a
    // method name (e.g. JS .update(), React state updates) and produces false
    // positives when it appears alone. SELECT / INSERT / DELETE / CREATE are
    // unambiguous SQL keywords.
    let upper = content.to_uppercase();
    let has_dml = ["SELECT ", "INSERT ", "DELETE ", "CREATE "]
        .iter()
        .any(|kw| upper.contains(kw));
    let has_structural = ["FROM ", "INTO ", "TABLE ", "SET ", "UPDATE ", "JOIN "]
        .iter()
        .any(|kw| upper.contains(kw));

    if !has_dml || !has_structural {
        return refs;
    }

    for cap in re.captures_iter(content) {
        // Skip matches that start on a comment line. This filters the most
        // common false positives such as `// UPDATE the state` or
        // `# UPDATE from docs`. It doesn't cover block comments, but eliminates
        // the overwhelming majority of noise.
        let match_start = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let line_start = content[..match_start]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let line_prefix = content[line_start..match_start].trim_start();
        if line_prefix.starts_with("//")
            || line_prefix.starts_with('#')
            || line_prefix.starts_with("--")
            || line_prefix.starts_with('*')
        {
            continue;
        }
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
// Task A2: Column-level reference extraction (cxpak 3.0.0)
// ---------------------------------------------------------------------------

/// SQL keywords / tokens that can appear in a projection or predicate position
/// but are not column names. Used to filter the candidate column set.
const SQL_NON_COLUMN_TOKENS: &[&str] = &[
    "select", "from", "where", "and", "or", "not", "null", "is", "in", "as", "on", "join", "inner",
    "left", "right", "outer", "full", "cross", "natural", "using", "group", "by", "order",
    "having", "limit", "offset", "union", "all", "distinct", "case", "when", "then", "else", "end",
    "true", "false", "like", "between", "asc", "desc", "count", "sum", "avg", "min", "max",
    "coalesce", "values", "set", "into", "insert", "update", "delete", "create", "table",
];

/// Extract `(table, column)` references from a string of (possibly embedded)
/// SQL, resolved against `schema_index`.
///
/// Precision policy (deterministic; documented in ADR-0174):
/// - The query's referenced tables are first detected via [`detect_embedded_sql`].
///   Only those tables (intersected with the schema) are candidates.
/// - **`SELECT *`** (or `SELECT t.*`): we cannot name the columns, so we **fan
///   out to every column of each detected table** and the caller marks these
///   edges `Inferred`. This is explicit over-attribution, never a silent drop.
/// - **Qualified `t.col`:** attributed to table `t` if `t` is a detected table
///   (by table name or — best-effort — any detected table that owns `col`).
/// - **Bare `col`:** attributed only if **exactly one** detected table owns a
///   column named `col`. If zero or more-than-one detected table owns it, the
///   reference is left **unresolved** (dropped) rather than mis-attributed.
/// - Columns not owned by any detected table are dropped (never invented).
///
/// Returns a sorted, deduplicated vector so the output is insertion-order
/// independent.
pub fn extract_embedded_column_refs(
    content: &str,
    schema_index: &SchemaIndex,
) -> Vec<(String, String)> {
    // Detected tables present in the schema.
    let detected: Vec<&str> = detect_embedded_sql(content)
        .into_iter()
        .map(|r| r.table_name)
        .filter(|t| schema_index.tables.contains_key(t))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|t| {
            // Borrow the schema's canonical key string for stable references.
            schema_index
                .tables
                .get_key_value(&t)
                .map(|(k, _)| k.as_str())
                .unwrap_or("")
        })
        .filter(|s| !s.is_empty())
        .collect();

    if detected.is_empty() {
        return Vec::new();
    }

    let mut out: BTreeSet<(String, String)> = BTreeSet::new();

    // Gather candidate column tokens (lowercased, possibly table-qualified).
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    let mut star = false;

    for cap in RE_SELECT_LIST.captures_iter(content) {
        let list = &cap[1];
        for tok in list.split(',') {
            let tok = tok.trim();
            if tok == "*" || tok.ends_with(".*") {
                star = true;
                continue;
            }
            // Strip aliasing (`col AS alias`) — keep the first identifier path.
            let first = tok.split_whitespace().next().unwrap_or(tok);
            collect_identifier(first, &mut candidates);
        }
    }
    for cap in RE_INSERT_COLS.captures_iter(content) {
        for tok in cap[1].split(',') {
            collect_identifier(tok.trim(), &mut candidates);
        }
    }
    for cap in RE_COL_PREDICATE.captures_iter(content) {
        collect_identifier(&cap[1], &mut candidates);
    }

    // SELECT * fan-out: every column of every detected table.
    if star {
        for table_name in &detected {
            if let Some(table) = schema_index.tables.get(*table_name) {
                for col in &table.columns {
                    out.insert((table_name.to_string(), col.name.clone()));
                }
            }
        }
    }

    // Resolve the named candidates.
    for cand in &candidates {
        let lower = cand.to_lowercase();
        if SQL_NON_COLUMN_TOKENS.contains(&lower.as_str()) {
            continue;
        }
        if let Some((qual, col)) = lower.split_once('.') {
            // Qualified `t.col`: attribute to detected table `t` if it owns `col`,
            // else any detected table that owns `col`.
            if let Some(t) = detected
                .iter()
                .find(|t| t.to_lowercase() == qual && table_owns_column(schema_index, t, col))
            {
                out.insert((t.to_string(), column_name_of(schema_index, t, col)));
            } else if let Some(t) = detected
                .iter()
                .find(|t| table_owns_column(schema_index, t, col))
            {
                out.insert((t.to_string(), column_name_of(schema_index, t, col)));
            }
        } else {
            // Bare `col`: attribute only if exactly one detected table owns it.
            let owners: Vec<&&str> = detected
                .iter()
                .filter(|t| table_owns_column(schema_index, t, &lower))
                .collect();
            if owners.len() == 1 {
                let t = owners[0];
                out.insert((t.to_string(), column_name_of(schema_index, t, &lower)));
            }
            // 0 owners → unknown column (dropped); >1 owners → ambiguous (dropped).
        }
    }

    out.into_iter().collect()
}

/// Push the leading identifier path of `tok` (e.g. `u.email` or `email`) into
/// `candidates`, lowercased. Ignores function calls and literals.
fn collect_identifier(tok: &str, candidates: &mut BTreeSet<String>) {
    let tok = tok.trim().trim_matches(|c| c == '(' || c == ')');
    // Reject anything that isn't an identifier or qualified identifier.
    if tok.is_empty() {
        return;
    }
    let valid = tok
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if !valid {
        return;
    }
    candidates.insert(tok.to_lowercase());
}

/// True when `table` (a schema key) owns a column named `col` (case-insensitive).
fn table_owns_column(schema_index: &SchemaIndex, table: &str, col: &str) -> bool {
    schema_index
        .tables
        .get(table)
        .map(|t| t.columns.iter().any(|c| c.name.to_lowercase() == col))
        .unwrap_or(false)
}

/// Return the canonical (as-declared) column name for `col` in `table`,
/// falling back to `col` if not found.
fn column_name_of(schema_index: &SchemaIndex, table: &str, col: &str) -> String {
    schema_index
        .tables
        .get(table)
        .and_then(|t| {
            t.columns
                .iter()
                .find(|c| c.name.to_lowercase() == col)
                .map(|c| c.name.clone())
        })
        .unwrap_or_else(|| col.to_string())
}

// ---------------------------------------------------------------------------
// Task 13: Build schema edges
// ---------------------------------------------------------------------------

/// Build typed edges that connect source files to schema artifacts.
///
/// Each edge carries an explicit [`EdgeConfidence`]: structurally-proven edges
/// (FK / ORM / view / function / migration / column→table anchor / ORM-field
/// column refs) are [`Extracted`][EdgeConfidence::Extracted]; heuristic
/// edges (embedded-SQL table refs, embedded-SQL column refs, `SELECT *`
/// fan-out) are [`Inferred`][EdgeConfidence::Inferred]. The 4-tuple lets the
/// graph builder stamp the right confidence per edge rather than deriving a
/// single value from the edge type.
///
/// Produces edges for:
/// - FK references between table definition files
/// - View → source table
/// - DB function → referenced table
/// - Embedded SQL in application code → table definition file
/// - Embedded SQL / ORM field → specific column node → table file (Task A2)
/// - ORM model file → table definition file
/// - Migration sequence (each migration → its predecessor)
pub fn build_schema_edges(
    files: &[crate::core_graph::IndexedFile],
    schema_index: &crate::schema::SchemaIndex,
) -> Vec<(String, String, EdgeType, EdgeConfidence)> {
    let mut edges: Vec<(String, String, EdgeType, EdgeConfidence)> = Vec::new();
    // Set of column nodes already anchored to their table file, so each
    // `col:table.col → table_file` anchor edge is emitted at most once
    // regardless of how many source files reference the column.
    let mut anchored_columns: HashSet<String> = HashSet::new();

    // Helper: push the `col:table.col → table_file` anchor edge once.
    let mut anchor_column = |edges: &mut Vec<(String, String, EdgeType, EdgeConfidence)>,
                             node: &str,
                             table_file: &str| {
        if anchored_columns.insert(node.to_string()) {
            edges.push((
                node.to_string(),
                table_file.to_string(),
                EdgeType::ColumnReference,
                // The column→table anchor is structurally proven: the column
                // belongs to a table whose definition file is known.
                EdgeConfidence::Extracted,
            ));
        }
    };

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
                            EdgeConfidence::Extracted,
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
                    EdgeConfidence::Extracted,
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
                    EdgeConfidence::Extracted,
                ));
            }
        }
    }

    // Embedded SQL edges (scan all files): table-level (EmbeddedSql) edges are
    // preserved exactly, AND column-level (ColumnReference) edges are added.
    for file in files {
        let lang = file.language.as_deref().unwrap_or("");
        // Skip SQL files themselves (they define tables, not embed SQL)
        if lang == "sql" {
            continue;
        }

        let mut file_tables: HashSet<String> = HashSet::new();
        // (table_name, column_name) pairs this file references at column
        // resolution, deduplicated.
        let mut file_columns: HashSet<(String, String)> = HashSet::new();

        let mut scan = |text: &str| {
            for sql_ref in detect_embedded_sql(text) {
                file_tables.insert(sql_ref.table_name);
            }
            for (table, column) in extract_embedded_column_refs(text, schema_index) {
                file_columns.insert((table, column));
            }
        };

        // Scan symbol bodies
        if let Some(pr) = &file.parse_result {
            for symbol in &pr.symbols {
                scan(&symbol.body);
            }
        }
        // Scan file content directly (module-level SQL, files without parse results)
        scan(&file.content);

        // Table-level edges (unchanged behavior).
        for table_name in &file_tables {
            if let Some(table) = schema_index.tables.get(table_name) {
                edges.push((
                    file.relative_path.clone(),
                    table.file_path.clone(),
                    EdgeType::EmbeddedSql,
                    EdgeConfidence::Inferred,
                ));
            }
        }

        // Column-level edges: file → col:table.col, plus the column→table anchor.
        for (table_name, column) in &file_columns {
            if let Some(table) = schema_index.tables.get(table_name) {
                let node = crate::schema::column_node_id(table_name, column);
                edges.push((
                    file.relative_path.clone(),
                    node.clone(),
                    EdgeType::ColumnReference,
                    // Embedded-SQL column refs are heuristic (regex-extracted).
                    EdgeConfidence::Inferred,
                ));
                anchor_column(&mut edges, &node, &table.file_path);
            }
        }
    }

    // ORM model → table edges, plus ORM-field → column edges.
    for model in schema_index.orm_models.values() {
        if let Some(table) = schema_index.tables.get(&model.table_name) {
            edges.push((
                model.file_path.clone(),
                table.file_path.clone(),
                EdgeType::OrmModel,
                EdgeConfidence::Extracted,
            ));

            // Map each ORM field to a column of the model's table. A field maps
            // to a column when its name matches a declared column (case-
            // insensitive). Relation fields (FKs to other models) are skipped:
            // they reference another model, not a scalar column of this table.
            for field in &model.fields {
                if field.is_relation {
                    continue;
                }
                let field_lower = field.name.to_lowercase();
                if table
                    .columns
                    .iter()
                    .any(|c| c.name.to_lowercase() == field_lower)
                {
                    let node = crate::schema::column_node_id(&model.table_name, &field.name);
                    edges.push((
                        model.file_path.clone(),
                        node.clone(),
                        EdgeType::ColumnReference,
                        // ORM field → column is structurally proven (declared field).
                        EdgeConfidence::Extracted,
                    ));
                    anchor_column(&mut edges, &node, &table.file_path);
                }
            }
        }
    }

    // Migration sequence edges
    for chain in &schema_index.migrations {
        for i in 1..chain.migrations.len() {
            edges.push((
                chain.migrations[i].file_path.clone(),
                chain.migrations[i - 1].file_path.clone(),
                EdgeType::MigrationSequence,
                EdgeConfidence::Extracted,
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
    use crate::core_graph::IndexedFile;
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
    fn test_detect_embedded_sql_update_alone_no_edge() {
        // UPDATE alone no longer qualifies as a DML keyword to prevent false
        // positives from method calls like `.update()`. A bare UPDATE query
        // without SELECT / INSERT / DELETE / CREATE does not produce an edge.
        let content = r#"db.run("UPDATE products SET price = 10 WHERE id = 1")"#;
        let refs = detect_embedded_sql(content);
        assert!(
            refs.is_empty(),
            "UPDATE alone must not trigger embedded SQL detection"
        );
    }

    #[test]
    fn test_detect_embedded_sql_update_with_select() {
        // UPDATE in a transaction that also contains SELECT does qualify.
        let content = "SELECT id FROM products; UPDATE products SET price = 10;";
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

    #[test]
    fn test_detect_embedded_sql_ts_comment_update_no_edge() {
        // A TypeScript file with only `// UPDATE the state` must not produce an edge.
        let content = "// UPDATE the state when the user clicks\nconst state = {};\n";
        let refs = detect_embedded_sql(content);
        assert!(
            refs.is_empty(),
            "comment-only UPDATE must not produce embedded SQL refs: {:?}",
            refs
        );
    }

    #[test]
    fn test_detect_embedded_sql_python_docstring_no_edge() {
        // Python docstring containing "Update from into_db" — no real SQL.
        let content = "def save():\n    \"\"\"Update from into_db\"\"\"\n    pass\n";
        let refs = detect_embedded_sql(content);
        assert!(
            refs.is_empty(),
            "docstring pseudo-SQL must not produce embedded SQL refs: {:?}",
            refs
        );
    }

    #[test]
    fn test_detect_embedded_sql_real_join_query_produces_edge() {
        let content = "SELECT * FROM users JOIN orders ON users.id = orders.user_id";
        let refs = detect_embedded_sql(content);
        let names: Vec<&str> = refs.iter().map(|r| r.table_name.as_str()).collect();
        assert!(names.contains(&"users"), "must find 'users': {:?}", names);
        assert!(names.contains(&"orders"), "must find 'orders': {:?}", names);
    }

    #[test]
    fn test_detect_embedded_sql_lazy_lock_consistent() {
        // Call detect_embedded_sql twice with the same content. Both calls must
        // return identical results, confirming the static regex is reused
        // rather than compiled fresh each invocation.
        let content = "SELECT * FROM users JOIN orders ON users.id = orders.user_id";
        let first = detect_embedded_sql(content);
        let second = detect_embedded_sql(content);

        let first_names: Vec<&str> = first.iter().map(|r| r.table_name.as_str()).collect();
        let second_names: Vec<&str> = second.iter().map(|r| r.table_name.as_str()).collect();

        assert_eq!(
            first_names, second_names,
            "two calls with same input must produce identical results"
        );
        assert!(first_names.contains(&"users"));
        assert!(first_names.contains(&"orders"));
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
                .any(|(from, to, etype, _)| from == "schema/orders.sql"
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
            edges
                .iter()
                .any(|(from, to, etype, _)| from == "src/repo.rs"
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
                .any(|(from, to, etype, _)| from == "db/migrate/002_add_email.rb"
                    && to == "db/migrate/001_create_users.rb"
                    && *etype == EdgeType::MigrationSequence),
            "should have edge 002→001: {:?}",
            edges
        );
        assert!(
            edges
                .iter()
                .any(|(from, to, etype, _)| from == "db/migrate/003_add_index.rb"
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
            edges
                .iter()
                .any(|(from, to, etype, _)| from == "app/models.py"
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
            edges.iter().any(|(from, to, etype, _)| from == "a.sql"
                && to == "b.sql"
                && *etype == EdgeType::ForeignKey),
            "should have FK a→b: {:?}",
            edges
        );
        assert!(
            edges.iter().any(|(from, to, etype, _)| from == "b.sql"
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
                .any(|(from, _, etype, _)| from == "other.sql" && *etype == EdgeType::EmbeddedSql),
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
            edges
                .iter()
                .any(|(from, to, etype, _)| from == "src/orders.ts"
                    && to == "db/schema.sql"
                    && *etype == EdgeType::EmbeddedSql),
            "should detect embedded SQL in symbol body: {:?}",
            edges
        );
    }
}
