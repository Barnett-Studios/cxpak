//! Task A2 — column-level lineage + column-granular blast radius.
//!
//! Contract: "alter `users.email`" must resolve to the specific queries / ORM
//! models / endpoints / tests that touch THAT column — and a blast for a
//! DIFFERENT column (`users.name`) must EXCLUDE files that only touch `email`.
//! This is the precision guarantee: changing one column must not fan out to
//! the whole table.
//!
//! These tests exercise the public surface added by A2:
//!   * `cxpak::schema::column_node_id` — stable, normalized column-node identity
//!   * `EdgeType::ColumnReference` — column-target edge type
//!   * `cxpak::schema::link::build_schema_edges` — now emits column edges
//!   * `cxpak::intelligence::blast_radius::compute_column_blast_radius` — column seed

use cxpak::core_graph::IndexedFile;
use cxpak::index::graph::build_dependency_graph;
use cxpak::intelligence::blast_radius::compute_column_blast_radius;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::schema::link::build_schema_edges;
use cxpak::schema::{
    column_node_id, ColumnSchema, EdgeConfidence, EdgeType, OrmFieldSchema, OrmFramework,
    OrmModelSchema, SchemaIndex, TableSchema,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn plain_col(name: &str) -> ColumnSchema {
    ColumnSchema {
        name: name.to_string(),
        data_type: "TEXT".to_string(),
        nullable: true,
        default: None,
        constraints: vec![],
        foreign_key: None,
    }
}

fn users_table() -> TableSchema {
    TableSchema {
        name: "users".to_string(),
        columns: vec![plain_col("id"), plain_col("email"), plain_col("name")],
        primary_key: Some(vec!["id".to_string()]),
        indexes: vec![],
        file_path: "schema/users.sql".to_string(),
        start_line: 1,
    }
}

fn indexed_file(path: &str, lang: &str, content: &str, symbols: Vec<Symbol>) -> IndexedFile {
    IndexedFile {
        relative_path: path.to_string(),
        language: Some(lang.to_string()),
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

fn symbol(name: &str, body: &str) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        signature: format!("fn {name}()"),
        body: body.to_string(),
        start_line: 1,
        end_line: 10,
    }
}

/// A schema + source fixture: a `users` table, a query selecting `email`, an
/// ORM model with an `email` field, an endpoint handler, and a test — plus a
/// SEPARATE file that only touches `name`.
fn fixture() -> (Vec<IndexedFile>, SchemaIndex) {
    let mut schema = SchemaIndex::empty();
    schema.tables.insert("users".to_string(), users_table());
    schema.orm_models.insert(
        "User".to_string(),
        OrmModelSchema {
            class_name: "User".to_string(),
            table_name: "users".to_string(),
            framework: OrmFramework::Django,
            file_path: "app/models.py".to_string(),
            fields: vec![
                OrmFieldSchema {
                    name: "id".to_string(),
                    field_type: "Integer".to_string(),
                    is_relation: false,
                    related_model: None,
                },
                OrmFieldSchema {
                    name: "email".to_string(),
                    field_type: "Char".to_string(),
                    is_relation: false,
                    related_model: None,
                },
            ],
        },
    );

    // Query file selecting the email column.
    let query_file = indexed_file(
        "src/repo/email_repo.rs",
        "rust",
        "",
        vec![symbol(
            "find_by_email",
            "db.query(\"SELECT id, email FROM users WHERE email = $1\")",
        )],
    );

    // Endpoint handler that also embeds an email query.
    let endpoint_file = indexed_file(
        "src/api/email_handler.rs",
        "rust",
        "db.query(\"SELECT email FROM users WHERE id = $1\");",
        vec![],
    );

    // Test file that exercises the email path.
    let test_file = indexed_file(
        "tests/email_test.rs",
        "rust",
        "let row = db.query(\"SELECT email FROM users WHERE id = 1\");",
        vec![],
    );

    // A DIFFERENT file that only touches the `name` column — must NOT appear in
    // an `email` blast.
    let name_file = indexed_file(
        "src/repo/name_repo.rs",
        "rust",
        "",
        vec![symbol(
            "find_by_name",
            "db.query(\"SELECT id, name FROM users WHERE name = $1\")",
        )],
    );

    (
        vec![query_file, endpoint_file, test_file, name_file],
        schema,
    )
}

// ---------------------------------------------------------------------------
// Column node identity
// ---------------------------------------------------------------------------

#[test]
fn column_node_id_is_normalized_and_namespaced() {
    // Case-insensitive (consistent with table keying) and prefixed so it can
    // never collide with a real file path.
    assert_eq!(column_node_id("Users", "Email"), "col:users.email");
    assert_eq!(column_node_id("users", "email"), "col:users.email");
    assert!(column_node_id("users", "email").starts_with("col:"));
    // Different columns produce different ids (collision-safe).
    assert_ne!(
        column_node_id("users", "email"),
        column_node_id("users", "name")
    );
    assert_ne!(
        column_node_id("users", "email"),
        column_node_id("orders", "email")
    );
}

// ---------------------------------------------------------------------------
// Edge construction from embedded SQL + ORM
// ---------------------------------------------------------------------------

#[test]
fn build_schema_edges_emits_column_edges_from_embedded_sql() {
    let (files, schema) = fixture();
    let edges = build_schema_edges(&files, &schema);

    let email_node = column_node_id("users", "email");

    // Query file → email column node, as a ColumnReference.
    assert!(
        edges
            .iter()
            .any(|(from, to, et, _)| from == "src/repo/email_repo.rs"
                && *to == email_node
                && *et == EdgeType::ColumnReference),
        "query file must reference users.email column node: {edges:?}"
    );

    // Column node → table definition file (anchors the column to its table).
    assert!(
        edges.iter().any(|(from, to, et, _)| *from == email_node
            && to == "schema/users.sql"
            && *et == EdgeType::ColumnReference),
        "column node must anchor to its table file: {edges:?}"
    );

    // Table-level embedded-SQL edge must STILL exist (no regression).
    assert!(
        edges
            .iter()
            .any(|(from, to, et, _)| from == "src/repo/email_repo.rs"
                && to == "schema/users.sql"
                && *et == EdgeType::EmbeddedSql),
        "table-level EmbeddedSql edge must be preserved: {edges:?}"
    );
}

#[test]
fn build_schema_edges_emits_column_edges_from_orm_fields() {
    let (files, schema) = fixture();
    let edges = build_schema_edges(&files, &schema);

    let email_node = column_node_id("users", "email");
    // ORM model file → email column node.
    assert!(
        edges.iter().any(|(from, to, et, _)| from == "app/models.py"
            && *to == email_node
            && *et == EdgeType::ColumnReference),
        "ORM model must reference its email field's column node: {edges:?}"
    );
    // The non-relation `id` field too.
    let id_node = column_node_id("users", "id");
    assert!(
        edges
            .iter()
            .any(|(from, to, _, _)| from == "app/models.py" && *to == id_node),
        "ORM model must reference its id field's column node: {edges:?}"
    );
}

#[test]
fn column_reference_edge_confidence_is_wired() {
    // Embedded-SQL-derived column refs are heuristic → Inferred.
    // ORM-field column refs are structural → Extracted.
    // We assert via the graph (which stamps confidence through default_confidence
    // for the generic case, but column edges carry explicit confidence).
    let (files, schema) = fixture();
    let graph = build_dependency_graph(&files, Some(&schema));
    let email_node = column_node_id("users", "email");

    let orm_edge = graph
        .edges
        .get("app/models.py")
        .and_then(|set| set.iter().find(|e| e.target == email_node));
    assert!(orm_edge.is_some(), "ORM→column edge must exist in graph");
    assert_eq!(
        orm_edge.unwrap().confidence,
        EdgeConfidence::Extracted,
        "ORM field column ref is structural → Extracted"
    );

    let sql_edge = graph
        .edges
        .get("src/repo/email_repo.rs")
        .and_then(|set| set.iter().find(|e| e.target == email_node));
    assert!(sql_edge.is_some(), "SQL→column edge must exist in graph");
    assert_eq!(
        sql_edge.unwrap().confidence,
        EdgeConfidence::Inferred,
        "embedded-SQL column ref is heuristic → Inferred"
    );
}

// ---------------------------------------------------------------------------
// SELECT * / unresolved handling
// ---------------------------------------------------------------------------

#[test]
fn select_star_fans_out_to_all_table_columns_as_inferred() {
    let mut schema = SchemaIndex::empty();
    schema.tables.insert("users".to_string(), users_table());

    let star_file = indexed_file(
        "src/repo/all_repo.rs",
        "rust",
        "db.query(\"SELECT * FROM users WHERE id = $1\");",
        vec![],
    );

    let edges = build_schema_edges(&[star_file], &schema);

    // SELECT * must fan out to every column of users (id, email, name),
    // explicitly — not silently dropped.
    for col in ["id", "email", "name"] {
        let node = column_node_id("users", col);
        assert!(
            edges
                .iter()
                .any(|(from, to, et, _)| from == "src/repo/all_repo.rs"
                    && *to == node
                    && *et == EdgeType::ColumnReference),
            "SELECT * must fan out to users.{col}: {edges:?}"
        );
    }

    // And the fan-out edges must be marked Inferred (not provably exact).
    let graph = build_dependency_graph(
        &[indexed_file(
            "src/repo/all_repo.rs",
            "rust",
            "db.query(\"SELECT * FROM users WHERE id = $1\");",
            vec![],
        )],
        Some(&schema),
    );
    let node = column_node_id("users", "email");
    let e = graph
        .edges
        .get("src/repo/all_repo.rs")
        .and_then(|set| set.iter().find(|e| e.target == node))
        .expect("SELECT * fan-out edge present");
    assert_eq!(
        e.confidence,
        EdgeConfidence::Inferred,
        "SELECT * fan-out edges are Inferred"
    );
}

#[test]
fn unattributable_column_is_not_misattributed() {
    // A column that does not belong to any known table must not produce a
    // bogus column node. `ghost` is not a column of `users`.
    let mut schema = SchemaIndex::empty();
    schema.tables.insert("users".to_string(), users_table());

    let f = indexed_file(
        "src/repo/ghost.rs",
        "rust",
        "db.query(\"SELECT ghost FROM users WHERE id = $1\");",
        vec![],
    );
    let edges = build_schema_edges(&[f], &schema);

    assert!(
        !edges
            .iter()
            .any(|(_, to, _, _)| *to == column_node_id("users", "ghost")),
        "unknown column `ghost` must not be attributed: {edges:?}"
    );
    // But a real column in the same query (none here) — and the table edge —
    // still resolve.
    assert!(
        edges
            .iter()
            .any(|(from, to, et, _)| from == "src/repo/ghost.rs"
                && to == "schema/users.sql"
                && *et == EdgeType::EmbeddedSql),
        "table-level edge still produced: {edges:?}"
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn column_edges_are_insertion_order_independent() {
    let (mut files, schema) = fixture();
    let edges_a = build_schema_edges(&files, &schema);

    files.reverse();
    let mut schema_b = SchemaIndex::empty();
    // Re-insert tables/models in a different order.
    schema_b.orm_models.extend(
        schema
            .orm_models
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    schema_b
        .tables
        .extend(schema.tables.iter().map(|(k, v)| (k.clone(), v.clone())));
    let edges_b = build_schema_edges(&files, &schema_b);

    // The graph (which sorts edges into BTreeSets) must be byte-identical
    // regardless of file/insertion order.
    let graph_a = build_dependency_graph(&files, Some(&schema));
    let graph_b = build_dependency_graph(&files, Some(&schema_b));
    let json_a = serde_json::to_string(&graph_a.edges).unwrap();
    let json_b = serde_json::to_string(&graph_b.edges).unwrap();
    assert_eq!(
        json_a, json_b,
        "graph edge serialization must be deterministic"
    );

    // Sorted edge tuples must match too.
    let mut sa: Vec<_> = edges_a
        .iter()
        .map(|(f, t, _, _)| (f.clone(), t.clone()))
        .collect();
    let mut sb: Vec<_> = edges_b
        .iter()
        .map(|(f, t, _, _)| (f.clone(), t.clone()))
        .collect();
    sa.sort();
    sb.sort();
    assert_eq!(sa, sb, "edge set must be order-independent");
}

// ---------------------------------------------------------------------------
// Contract: "alter users.email" — column-granular blast radius
// ---------------------------------------------------------------------------

fn empty_pagerank(graph: &cxpak::core_graph::graph::DependencyGraph) -> HashMap<String, f64> {
    // Give every node a uniform non-zero pagerank so risk scores are non-zero
    // and files are not dropped for lack of importance.
    let mut pr = HashMap::new();
    for k in graph.edges.keys().chain(graph.reverse_edges.keys()) {
        pr.insert(k.clone(), 0.5);
    }
    pr
}

#[test]
fn alter_users_email_blast_includes_email_touchers() {
    let (files, schema) = fixture();
    let graph = build_dependency_graph(&files, Some(&schema));
    let pagerank = empty_pagerank(&graph);
    let test_map: HashMap<String, Vec<_>> = HashMap::new();

    let result =
        compute_column_blast_radius("users", "email", &graph, &pagerank, &test_map, 5, None);

    let all: Vec<&str> = result
        .categories
        .direct_dependents
        .iter()
        .chain(result.categories.transitive_dependents.iter())
        .chain(result.categories.schema_dependents.iter())
        .chain(result.categories.test_files.iter())
        .map(|a| a.path.as_str())
        .collect();

    for expected in [
        "src/repo/email_repo.rs",
        "src/api/email_handler.rs",
        "tests/email_test.rs",
        "app/models.py",
    ] {
        assert!(
            all.contains(&expected),
            "email blast must include {expected}; got {all:?}"
        );
    }
}

#[test]
fn alter_users_name_blast_excludes_email_only_files() {
    let (files, schema) = fixture();
    let graph = build_dependency_graph(&files, Some(&schema));
    let pagerank = empty_pagerank(&graph);
    let test_map: HashMap<String, Vec<_>> = HashMap::new();

    let result =
        compute_column_blast_radius("users", "name", &graph, &pagerank, &test_map, 5, None);

    let all: Vec<&str> = result
        .categories
        .direct_dependents
        .iter()
        .chain(result.categories.transitive_dependents.iter())
        .chain(result.categories.schema_dependents.iter())
        .chain(result.categories.test_files.iter())
        .map(|a| a.path.as_str())
        .collect();

    // Precision: a `name` blast must reach the name file...
    assert!(
        all.contains(&"src/repo/name_repo.rs"),
        "name blast must include the name file; got {all:?}"
    );
    // ...but NOT the email-only files (the whole point of column granularity).
    for forbidden in [
        "src/repo/email_repo.rs",
        "src/api/email_handler.rs",
        "tests/email_test.rs",
    ] {
        assert!(
            !all.contains(&forbidden),
            "name blast must EXCLUDE email-only file {forbidden}; got {all:?}"
        );
    }
}
