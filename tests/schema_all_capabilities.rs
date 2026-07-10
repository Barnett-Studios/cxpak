// Task 0.5 — Versioned per-capability schema registry integration test
//
// Verifies:
// 1. Each known capability id (context/graph/data/review) returns Ok with a
//    valid JSON object carrying `x-format-version` and `$schema` (draft 2020-12),
//    serializable via serde_json::to_string_pretty.
//    NOTE: only the `context` capability's output type (AutoContextResult)
//    carries a `format_version` field — graph/data/review schemas do NOT
//    include `format_version` in `required` because the Rust types they
//    describe do not emit it (ADR-0097 descriptive-honesty).
// 2. An unknown id returns Err (not a silent default).
// 3. The `graph` schema includes the edge `confidence` field with
//    Extracted/Inferred variants — proving 0.4 linkage.
// 4. The `context` capability still advertises "2.3" (auto_context contract
//    unchanged by 0.5).
// 5. Schema↔type correspondence: every key in a capability schema's `required`
//    array is present in the serialized output of a real instance (schema-required
//    ⊆ actual-fields). Enforces the no-aspirational-fields invariant.

use cxpak::auto_context::diff::{ContextDelta, FileChange, GraphChange, SymbolChange};
use cxpak::commands::schema::capability_schema;
use cxpak::core_graph::graph::{DependencyGraph, EdgeType};
use cxpak::core_graph::schema::SchemaIndex;
use serde_json::json;

#[test]
fn context_capability_returns_ok_with_required_fields() {
    let schema = capability_schema("context").expect("context capability must return Ok");
    assert!(schema.is_object(), "schema must be a JSON object");
    assert!(
        schema["$schema"].is_string(),
        "schema must have a $schema field"
    );
    assert!(
        schema["$schema"].as_str().unwrap().contains("2020-12"),
        "schema must use draft 2020-12"
    );
    assert!(
        schema["format_version"].is_string() || schema["properties"]["format_version"].is_object(),
        "schema must expose format_version"
    );
    // The context capability must remain at "2.3" — auto_context output unchanged.
    assert_eq!(
        schema["x-format-version"],
        json!("2.3"),
        "context capability must stay at format version 2.3"
    );
    assert!(serde_json::to_string_pretty(&schema).is_ok());
}

#[test]
fn graph_capability_returns_ok_with_required_fields() {
    let schema = capability_schema("graph").expect("graph capability must return Ok");
    assert!(schema.is_object());
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    assert!(schema["x-format-version"].is_string());
    assert!(serde_json::to_string_pretty(&schema).is_ok());
}

#[test]
fn graph_schema_describes_edge_confidence() {
    // Proves Task 0.4 linkage: the graph schema must document the confidence
    // field with Extracted and Inferred variants.
    let schema = capability_schema("graph").expect("graph capability must return Ok");
    let schema_str = serde_json::to_string_pretty(&schema).unwrap();
    assert!(
        schema_str.contains("confidence"),
        "graph schema must mention 'confidence' field"
    );
    assert!(
        schema_str.contains("Extracted"),
        "graph schema must mention 'Extracted' confidence variant"
    );
    assert!(
        schema_str.contains("Inferred"),
        "graph schema must mention 'Inferred' confidence variant"
    );
}

#[test]
fn data_capability_returns_ok_with_required_fields() {
    let schema = capability_schema("data").expect("data capability must return Ok");
    assert!(schema.is_object());
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    assert!(schema["x-format-version"].is_string());
    assert!(serde_json::to_string_pretty(&schema).is_ok());
}

#[test]
fn review_capability_returns_ok_with_required_fields() {
    let schema = capability_schema("review").expect("review capability must return Ok");
    assert!(schema.is_object());
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    assert!(schema["x-format-version"].is_string());
    assert!(serde_json::to_string_pretty(&schema).is_ok());
}

#[test]
fn unknown_capability_id_returns_err() {
    let result = capability_schema("nope");
    assert!(
        result.is_err(),
        "unknown capability id must return Err, not a silent default"
    );
    let result2 = capability_schema("bogus_capability_xyz");
    assert!(result2.is_err());
}

#[test]
fn all_capabilities_have_schema_draft_2020_12() {
    for id in &["context", "graph", "data", "review"] {
        let schema =
            capability_schema(id).unwrap_or_else(|_| panic!("capability '{}' must return Ok", id));
        let schema_url = schema["$schema"]
            .as_str()
            .unwrap_or_else(|| panic!("capability '{}' must have a $schema string", id));
        assert!(
            schema_url.contains("2020-12"),
            "capability '{}' must use JSON Schema draft 2020-12, got: {}",
            id,
            schema_url
        );
    }
}

// ---------------------------------------------------------------------------
// Schema↔type correspondence tests (ADR-0097 descriptive-honesty guard)
//
// For each of graph/data/review: construct a real instance, serialize it, and
// assert that every key listed in the capability schema's `required` array is
// present in the serialized object.  This makes "required ⊆ actual-fields" a
// hard enforced invariant — the bug that prompted this test (format_version in
// required but absent from the type) cannot silently re-enter.
//
// AutoContextResult requires a full pipeline run to construct; its correspondence
// is implicitly covered by the existing spa_determinism fixture test.
// ---------------------------------------------------------------------------

fn schema_required_keys(schema: &serde_json::Value) -> Vec<String> {
    schema["required"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn graph_schema_required_fields_present_in_real_instance() {
    // Build a real DependencyGraph with one edge so both fields are populated.
    let mut graph = DependencyGraph::new();
    graph.add_edge("src/a.rs", "src/b.rs", EdgeType::Import);

    let actual = serde_json::to_value(&graph).expect("DependencyGraph must serialize");
    let schema = capability_schema("graph").expect("graph capability must return Ok");

    for key in schema_required_keys(&schema) {
        assert!(
            actual.get(&key).is_some(),
            "graph schema requires '{}' but DependencyGraph does not emit it — schema is aspirational",
            key
        );
    }
}

#[test]
fn data_schema_required_fields_present_in_real_instance() {
    // Use SchemaIndex::empty() — all maps/vecs present (just empty).
    let index = SchemaIndex::empty();
    let actual = serde_json::to_value(&index).expect("SchemaIndex must serialize");
    let schema = capability_schema("data").expect("data capability must return Ok");

    for key in schema_required_keys(&schema) {
        assert!(
            actual.get(&key).is_some(),
            "data schema requires '{}' but SchemaIndex does not emit it — schema is aspirational",
            key
        );
    }
}

#[test]
fn review_schema_required_fields_present_in_real_instance() {
    // Construct a minimal ContextDelta literal.
    let delta = ContextDelta {
        modified_files: vec![FileChange {
            path: "src/a.rs".to_string(),
            change: "modified".to_string(),
            tokens_delta: 10,
        }],
        new_files: vec![],
        deleted_files: vec![],
        new_symbols: vec![SymbolChange {
            path: "src/a.rs".to_string(),
            name: "foo".to_string(),
            kind: "Function".to_string(),
        }],
        removed_symbols: vec![],
        graph_changes: vec![GraphChange {
            change_type: "Added".to_string(),
            from: "src/a.rs".to_string(),
            to: "src/b.rs".to_string(),
            edge_type: "Import".to_string(),
        }],
        recommendation: "review changes".to_string(),
    };
    let actual = serde_json::to_value(&delta).expect("ContextDelta must serialize");
    let schema = capability_schema("review").expect("review capability must return Ok");

    for key in schema_required_keys(&schema) {
        assert!(
            actual.get(&key).is_some(),
            "review schema requires '{}' but ContextDelta does not emit it — schema is aspirational",
            key
        );
    }
}
