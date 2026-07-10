//! Surface conformance gate (generalizes ADR-0153 from the SPA-only check to
//! all declared surfaces).
//!
//! Invariant: a surface projection is a *reshaping* of the core function's
//! output, never a re-derivation. So for every capability in `catalog()`, for
//! every surface that capability declares, the surface's projected data must
//! round-trip EQUAL to the core result.
//!
//! Technique (from `tests/cross_channel_consistency.rs`): `serde_json`
//! round-trip equality, with `f64::to_bits()` for float/composite fields so we
//! catch third-decimal drift and HashMap-ordering drift. A capability that does
//! NOT declare a surface is intentionally not tested for that surface (the
//! dashed cell in the capability × surface matrix).
//!
//! # Scope of this gate (read before trusting a green run)
//!
//! This gate validates the projection framework's round-trip **invertibility**
//! using STUB core values: it asserts that `recover_core(project(core)) == core`
//! for every declared (capability, surface) cell. That is the correct
//! foundational scope for Task 0.6 — the framework is parallel, and live
//! surfaces do not yet route through it.
//!
//! It deliberately does NOT validate that a declared surface actually exposes
//! the capability today. The test iterates the catalog's OWN declared bits and
//! round-trips a stub through identity project/recover, so an *aspirational*
//! surface bit would still pass green. Surface honesty is enforced separately by
//! keeping the catalog's `projections` bits descriptive (only `true` where a
//! surface genuinely returns that capability's data today — see
//! `src/capability/mod.rs`).
//!
//! When B1/C3 route live surfaces through the adapter, this harness should swap
//! the stub `sample_core` for real `intelligence::*` results and add a
//! per-(capability, surface) **reachability** assertion (the surface must
//! actually produce the capability's data). Until then: a green run here means
//! "the projection framework is invertible", NOT "every declared surface is
//! verified to exist".

use cxpak::capability::adapter::{project, recover_core, Surface, ALL_SURFACES};
use cxpak::capability::{catalog, Capability};
use serde_json::{json, Value};

/// Build a small deterministic core result for a capability id. The *value*
/// here stands in for "the single core result that all surfaces share"; the
/// test asserts each surface projection of it recovers an equal value. We
/// deliberately include a float and a nested map so the bit-level + ordering
/// checks have something to bite on.
fn sample_core(id: &str) -> Value {
    match id {
        "review" => json!({
            "recommendation": "review src/a.rs first",
            "modified_files": [{"path": "src/a.rs", "tokens_delta": -3}],
        }),
        // The float / nested-map shape below is shared by the remaining
        // capabilities; the conformance property is structural, not per-id.
        _ => json!({
            "id": id,
            "composite": 7.123_456_789_f64,
            "nested": {"b": 2, "a": 1, "score": 0.333_333_333_333_f64},
            "items": [{"path": "src/z.rs", "weight": 1.5}, {"path": "src/a.rs", "weight": 0.25}],
        }),
    }
}

/// Compare two JSON values for *bit-exact* equality, descending into objects
/// and arrays and comparing any f64 leaves via `to_bits()` (so 0.1+0.2 style
/// third-decimal drift and `-0.0` vs `0.0` are caught). Object key ordering is
/// irrelevant because we look keys up by name — but a *missing* or *extra* key
/// fails.
fn bit_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(fx), Some(fy)) => fx.to_bits() == fy.to_bits(),
            _ => x == y,
        },
        (Value::Object(ox), Value::Object(oy)) => {
            ox.len() == oy.len()
                && ox
                    .iter()
                    .all(|(k, vx)| oy.get(k).is_some_and(|vy| bit_equal(vx, vy)))
        }
        (Value::Array(ax), Value::Array(ay)) => {
            ax.len() == ay.len() && ax.iter().zip(ay).all(|(vx, vy)| bit_equal(vx, vy))
        }
        _ => a == b,
    }
}

fn declares(cap: &Capability, surface: Surface) -> bool {
    match surface {
        Surface::Mcp => cap.projections.mcp,
        Surface::Lsp => cap.projections.lsp,
        Surface::Cli => cap.projections.cli,
        Surface::Http => cap.projections.http,
        Surface::Visual => cap.projections.visual,
    }
}

#[test]
fn every_declared_surface_round_trips_to_core() {
    let mut tested_cells = 0usize;
    for cap in catalog() {
        let core = sample_core(cap.id);
        for &surface in ALL_SURFACES {
            if !declares(cap, surface) {
                continue; // dashed cell — intentionally untested
            }
            tested_cells += 1;
            let projected = project(cap, surface, &core);
            let recovered = recover_core(cap, surface, &projected);
            assert!(
                bit_equal(&core, &recovered),
                "conformance violation: capability `{}` on {surface:?} did not \
                 round-trip equal to its core result.\n  core:      {core}\n  \
                 projected: {projected}\n  recovered: {recovered}",
                cap.id
            );
        }
    }
    assert!(
        tested_cells > 0,
        "catalog declared no surfaces — nothing was conformance-checked"
    );
}

#[test]
fn projection_is_pure_no_recompute() {
    // A projection must depend ONLY on the core value passed in: projecting two
    // different cores must yield two recoveries equal to their respective cores
    // (i.e. the surface never substitutes its own derivation). This pins the
    // ADR-0153 single-source invariant at the framework boundary.
    for cap in catalog() {
        let core_a = sample_core(cap.id);
        let core_b = json!({"sentinel": 42, "x": [core_a.clone()]});
        for &surface in ALL_SURFACES {
            if !declares(cap, surface) {
                continue;
            }
            let rec_a = recover_core(cap, surface, &project(cap, surface, &core_a));
            let rec_b = recover_core(cap, surface, &project(cap, surface, &core_b));
            assert!(bit_equal(&core_a, &rec_a), "{} {surface:?} core_a", cap.id);
            assert!(bit_equal(&core_b, &rec_b), "{} {surface:?} core_b", cap.id);
        }
    }
}

// ---------------------------------------------------------------------------
// C3 (ADR-0182) / B1 M2 extension: real-core reachability for the migrated
// `graph` and `data` ops.
//
// The gate above proves the projection framework is *invertible* over stub
// cores. This extension proves the stronger property the C3 migration requires
// for `graph` and `data`: their MCP op now routes to a REAL `intelligence` /
// `SchemaIndex` core on the live surface, and that real core still round-trips
// bit-equal through every surface the capability declares.
// ---------------------------------------------------------------------------

use cxpak::budget::counter::TokenCounter;
use cxpak::commands::serve::handle_tool_call;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{Import, ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

fn real_index() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let files = vec![
        ScannedFile {
            relative_path: "src/a.rs".into(),
            absolute_path: "/tmp/src/a.rs".into(),
            language: Some("rust".into()),
            size_bytes: 100,
        },
        ScannedFile {
            relative_path: "src/b.rs".into(),
            absolute_path: "/tmp/src/b.rs".into(),
            language: Some("rust".into()),
            size_bytes: 100,
        },
        ScannedFile {
            relative_path: "db/t.sql".into(),
            absolute_path: "/tmp/db/t.sql".into(),
            language: Some("sql".into()),
            size_bytes: 80,
        },
    ];
    let mut pr = HashMap::new();
    pr.insert(
        "src/a.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "f".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "pub fn f()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 2,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    pr.insert(
        "src/b.rs".to_string(),
        ParseResult {
            symbols: vec![],
            imports: vec![Import {
                source: "crate::a".into(),
                names: vec!["f".into()],
            }],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/a.rs".to_string(), "pub fn f(){}".into());
    content.insert("src/b.rs".to_string(), "use crate::a::f;".into());
    content.insert(
        "db/t.sql".to_string(),
        "CREATE TABLE t (id INTEGER PRIMARY KEY);".into(),
    );
    CodebaseIndex::build_with_content(files, pr, &counter, content)
}

/// Extract the real capability core the live MCP op returned (unwrap the tool
/// envelope's `result.content[0].text` back to JSON).
fn live_core(idx: &CodebaseIndex, tool: &str, args: Value) -> Value {
    let snap: cxpak::commands::serve::SharedSnapshot = Arc::new(RwLock::new(None));
    let resp = handle_tool_call(
        Some(json!(1)),
        tool,
        &args,
        idx,
        std::path::Path::new("/tmp"),
        &snap,
    );
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no core for {tool}: {resp}"));
    serde_json::from_str(text).unwrap()
}

fn cap_for(id: &str) -> &'static Capability {
    catalog().iter().find(|c| c.id == id).expect("present")
}

#[test]
fn migrated_graph_op_real_core_round_trips_all_surfaces() {
    let idx = real_index();
    // Real graph-query core (not a stub) via the live MCP `graph` op.
    let core = live_core(
        &idx,
        "cxpak_graph",
        json!({"op": "graph", "graph_op": "neighbors", "id": "src/b.rs", "direction": "both"}),
    );
    let neighbors = core["neighbors"].as_array().expect("neighbors");
    assert!(
        !neighbors.is_empty(),
        "b.rs imports a.rs — expected an edge"
    );
    // A3: the real core carries per-edge confidence.
    assert!(neighbors[0]["confidence"].is_string());

    // The real core round-trips bit-equal through every surface graph declares.
    let cap = cap_for("graph");
    for &surface in ALL_SURFACES {
        if !declares(cap, surface) {
            continue;
        }
        let projected = project(cap, surface, &core);
        let recovered = recover_core(cap, surface, &projected);
        assert!(
            bit_equal(&core, &recovered),
            "graph real core did not round-trip on {surface:?}"
        );
    }
}

#[test]
fn migrated_data_op_real_core_round_trips_all_surfaces() {
    let idx = real_index();
    // Real SchemaIndex-derived core via the live MCP `data` op.
    let core = live_core(&idx, "cxpak_data", json!({"op": "data"}));
    assert!(
        core["indexed"].is_boolean(),
        "data core must be real: {core}"
    );
    assert!(core["tables"].is_array());

    let cap = cap_for("data");
    for &surface in ALL_SURFACES {
        if !declares(cap, surface) {
            continue;
        }
        let projected = project(cap, surface, &core);
        let recovered = recover_core(cap, surface, &projected);
        assert!(
            bit_equal(&core, &recovered),
            "data real core did not round-trip on {surface:?}"
        );
    }
}
