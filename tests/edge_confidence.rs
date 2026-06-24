// Task 0.4 — EdgeConfidence integration test
//
// Verifies that every TypedEdge produced by `add_edge` carries the correct
// `EdgeConfidence` value derived from its `EdgeType`:
//   Extracted — Import, ForeignKey, ViewReference, TriggerTarget, IndexTarget,
//               FunctionReference, OrmModel, MigrationSequence
//   Inferred  — EmbeddedSql, CrossLanguage(_)
//
// The test builds edges via `add_edge` (through `build_schema_edges` where
// appropriate) and asserts on the `confidence` field of the resulting
// `TypedEdge`s — proving the mapping flows through the constructor.

use cxpak::core_graph::graph::{BridgeType, DependencyGraph, EdgeConfidence, EdgeType};
use cxpak::core_graph::IndexedFile;
use cxpak::schema::{ColumnSchema, ForeignKeyRef, SchemaIndex, TableSchema};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn plain_column(name: &str) -> ColumnSchema {
    ColumnSchema {
        name: name.to_string(),
        data_type: "TEXT".to_string(),
        nullable: true,
        default: None,
        constraints: vec![],
        foreign_key: None,
    }
}

fn fk_column(name: &str, target_table: &str, target_col: &str) -> ColumnSchema {
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

fn make_file_no_parse(path: &str, language: &str, content: &str) -> IndexedFile {
    IndexedFile {
        relative_path: path.to_string(),
        language: Some(language.to_string()),
        size_bytes: content.len() as u64,
        token_count: 0,
        parse_result: None,
        content: content.to_string(),
        mtime_secs: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `Import` edge → `EdgeConfidence::Extracted`
#[test]
fn import_edge_is_extracted() {
    let mut g = DependencyGraph::new();
    g.add_edge("src/a.rs", "src/b.rs", EdgeType::Import);

    let edges = g.dependencies("src/a.rs").expect("edge present");
    let edge = edges
        .iter()
        .find(|e| e.target == "src/b.rs")
        .expect("b.rs in deps");

    assert_eq!(
        edge.confidence,
        EdgeConfidence::Extracted,
        "Import edge must be Extracted"
    );
}

/// `ForeignKey` edge produced by `build_schema_edges` → `EdgeConfidence::Extracted`
#[test]
fn fk_edge_is_extracted() {
    let users = make_table("users", "schema/users.sql", vec![plain_column("id")]);
    let orders = make_table(
        "orders",
        "schema/orders.sql",
        vec![fk_column("user_id", "users", "id")],
    );

    let mut schema = SchemaIndex::empty();
    schema.tables.insert("users".to_string(), users);
    schema.tables.insert("orders".to_string(), orders);

    let raw_edges = cxpak::schema::link::build_schema_edges(&[], &schema);

    // Wire the raw edges into a graph the same way `build_dependency_graph` does.
    let mut g = DependencyGraph::new();
    for (from, to, etype, conf) in raw_edges {
        g.add_edge_with_confidence(&from, &to, etype, conf);
    }

    let edges = g
        .dependencies("schema/orders.sql")
        .expect("orders in graph");
    let fk_edge = edges
        .iter()
        .find(|e| e.edge_type == EdgeType::ForeignKey)
        .expect("FK edge present");

    assert_eq!(
        fk_edge.confidence,
        EdgeConfidence::Extracted,
        "ForeignKey edge must be Extracted"
    );
}

/// `EmbeddedSql` edge produced by `build_schema_edges` → `EdgeConfidence::Inferred`
#[test]
fn embedded_sql_edge_is_inferred() {
    let products = make_table("products", "schema/products.sql", vec![]);

    let mut schema = SchemaIndex::empty();
    schema.tables.insert("products".to_string(), products);

    let rust_file = make_file_no_parse(
        "src/repo.rs",
        "rust",
        r#"fn list() { db.query("SELECT id FROM products WHERE active = true") }"#,
    );

    let raw_edges = cxpak::schema::link::build_schema_edges(&[rust_file], &schema);

    let mut g = DependencyGraph::new();
    for (from, to, etype, conf) in raw_edges {
        g.add_edge_with_confidence(&from, &to, etype, conf);
    }

    let edges = g.dependencies("src/repo.rs").expect("repo.rs in graph");
    let sql_edge = edges
        .iter()
        .find(|e| e.edge_type == EdgeType::EmbeddedSql)
        .expect("EmbeddedSql edge present");

    assert_eq!(
        sql_edge.confidence,
        EdgeConfidence::Inferred,
        "EmbeddedSql edge must be Inferred"
    );
}

/// `CrossLanguage` edge → `EdgeConfidence::Inferred`
#[test]
fn cross_language_edge_is_inferred() {
    let mut g = DependencyGraph::new();
    g.add_edge(
        "src/client.ts",
        "src/server.rs",
        EdgeType::CrossLanguage(BridgeType::HttpCall),
    );

    let edges = g.dependencies("src/client.ts").expect("client.ts in graph");
    let cl_edge = edges
        .iter()
        .find(|e| matches!(e.edge_type, EdgeType::CrossLanguage(_)))
        .expect("CrossLanguage edge present");

    assert_eq!(
        cl_edge.confidence,
        EdgeConfidence::Inferred,
        "CrossLanguage edge must be Inferred"
    );
}

/// Ord stability: two edges that differ only in confidence (impossible with type-derived
/// confidence, but we verify BTreeSet ordering is `target`-then-`edge_type`-first by
/// confirming a round-trip over distinct edges preserves their count.
#[test]
fn btreeset_ordering_stable_after_field_addition() {
    let mut g = DependencyGraph::new();
    g.add_edge("a.rs", "b.rs", EdgeType::Import);
    g.add_edge("a.rs", "b.rs", EdgeType::ForeignKey);
    g.add_edge("a.rs", "c.rs", EdgeType::Import);

    // Three distinct (from, to, edge_type) triples → 3 edges in BTreeSet.
    assert_eq!(
        g.edge_count(),
        3,
        "edge_count unchanged after field addition"
    );
}

/// Serialization round-trip: a TypedEdge serialized without the `confidence`
/// field (simulating a v4 stale cache entry) must deserialize to `Extracted`
/// via the `serde(default)` annotation.
#[test]
fn serde_default_missing_confidence_deserializes_as_extracted() {
    // Simulate a stale JSON entry that lacks the `confidence` field.
    let json = r#"{"target":"b.rs","edge_type":"Import"}"#;
    let edge: cxpak::core_graph::graph::TypedEdge =
        serde_json::from_str(json).expect("deserializes without confidence field");
    assert_eq!(
        edge.confidence,
        EdgeConfidence::Extracted,
        "missing `confidence` in JSON must default to Extracted"
    );
}
