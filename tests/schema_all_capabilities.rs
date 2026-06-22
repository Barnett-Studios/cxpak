// Task 0.5 — Versioned per-capability schema registry integration test
//
// RED phase: these tests will fail until `capability_schema` is implemented.
//
// Verifies:
// 1. Each known capability id (context/graph/data/review) returns Ok with a
//    valid JSON object carrying `format_version` and `$schema` (draft 2020-12),
//    serializable via serde_json::to_string_pretty.
// 2. An unknown id returns Err (not a silent default).
// 3. The `graph` schema includes the edge `confidence` field with
//    Extracted/Inferred variants — proving 0.4 linkage.
// 4. The `context` capability still advertises "2.3" (auto_context contract
//    unchanged by 0.5).

use cxpak::commands::schema::capability_schema;
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
