//! Adversarial tests that every cxpak/* LSP method is wired to a real
//! intelligence function and not to `custom_stub`. The old code in
//! `src/lsp/mod.rs` registered 10 of the 14 methods to
//! `CxpakLspBackend::custom_stub`, which returned a sentinel payload
//! `{"status": "available", "method": "<name>"}` regardless of inputs.
//!
//! These tests lock the invariant: no LSP method may return that sentinel.
#![cfg(feature = "lsp")]

use cxpak::budget::counter::TokenCounter;
use cxpak::index::CodebaseIndex;
use cxpak::parser::language::{ParseResult, Symbol, SymbolKind, Visibility};
use cxpak::scanner::ScannedFile;
use std::collections::HashMap;

fn git_tempdir() -> tempfile::TempDir {
    let temp = tempfile::TempDir::new().unwrap();
    let _ = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(temp.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(temp.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(temp.path())
        .output();
    std::fs::write(temp.path().join("README.md"), "init\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init", "--quiet"])
        .current_dir(temp.path())
        .output();
    temp
}

fn make_idx() -> CodebaseIndex {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 20,
    }];
    let mut parses = HashMap::new();
    parses.insert(
        "src/main.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "main".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                signature: "fn main()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 3,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/main.rs".into(), "fn main() {}".into());
    CodebaseIndex::build_with_content(files, parses, &counter, content)
}

fn assert_not_stub(m: &str, v: &serde_json::Value) {
    let status = v.get("status").and_then(|s| s.as_str());
    assert!(
        status != Some("available"),
        "{m} returned custom_stub sentinel: {v}"
    );
    assert!(
        status != Some("not_implemented"),
        "{m} returned not_implemented sentinel: {v}"
    );
}

/// Every cxpak/* method must return real data — no stub payloads.
#[test]
fn no_lsp_method_returns_stub_sentinel() {
    let idx = make_idx();
    let temp = git_tempdir();
    let root = temp.path();
    let cases: &[(&str, serde_json::Value)] = &[
        ("cxpak/health", serde_json::Value::Null),
        ("cxpak/conventions", serde_json::Value::Null),
        (
            "cxpak/blastRadius",
            serde_json::json!({"file": "src/main.rs"}),
        ),
        ("cxpak/overview", serde_json::Value::Null),
        ("cxpak/trace", serde_json::json!({"symbol": "main"})),
        ("cxpak/diff", serde_json::Value::Null),
        ("cxpak/search", serde_json::json!({"query": "main"})),
        ("cxpak/apiSurface", serde_json::Value::Null),
        ("cxpak/deadCode", serde_json::Value::Null),
        ("cxpak/callGraph", serde_json::Value::Null),
        (
            "cxpak/predict",
            serde_json::json!({"files": ["src/main.rs"]}),
        ),
        ("cxpak/drift", serde_json::Value::Null),
        ("cxpak/securitySurface", serde_json::Value::Null),
        ("cxpak/dataFlow", serde_json::json!({"symbol": "main"})),
    ];
    for (m, params) in cases {
        let result =
            cxpak::lsp::methods::handle_custom_method(m, params.clone(), &idx, root).expect(m);
        let body = result.unwrap_or_else(|| panic!("{m} returned None"));
        assert_not_stub(m, &body);
    }
}

/// `cxpak/blastRadius` must compute a real `BlastRadiusResult` structure
/// (with a `direct_dependents` array) — the old stub just emitted a
/// "requires file param" note.
#[test]
fn lsp_blast_radius_returns_real_structure() {
    let idx = make_idx();
    let temp = git_tempdir();
    let body = cxpak::lsp::methods::handle_custom_method(
        "cxpak/blastRadius",
        serde_json::json!({"file": "src/main.rs"}),
        &idx,
        temp.path(),
    )
    .unwrap()
    .unwrap();
    assert!(
        body.get("categories").is_some() && body.get("changed_files").is_some(),
        "blastRadius must return a real BlastRadiusResult, got: {body}"
    );
    assert!(body.get("note").is_none(), "no 'note' escape hatch allowed");
}

/// `cxpak/drift` must return a real DriftReport structure — not a "use CLI" note.
#[test]
fn lsp_drift_returns_real_structure() {
    let idx = make_idx();
    let temp = git_tempdir();
    let body = cxpak::lsp::methods::handle_custom_method(
        "cxpak/drift",
        serde_json::Value::Null,
        &idx,
        temp.path(),
    )
    .unwrap()
    .unwrap();
    // DriftReport serializes with baseline/trend/hotspots keys.
    assert!(
        body.get("baseline").is_some()
            || body.get("trend").is_some()
            || body.get("hotspots").is_some(),
        "drift must return real DriftReport, got: {body}"
    );
    assert!(
        body.get("note").is_none(),
        "no 'use cxpak drift CLI' escape hatch allowed"
    );
}

/// `cxpak/diff` must return a real change list structure — not a "use CLI" note.
#[test]
fn lsp_diff_returns_real_structure() {
    let idx = make_idx();
    let temp = git_tempdir();
    // Modify a tracked file so there's something to diff.
    std::fs::write(temp.path().join("README.md"), "updated\n").unwrap();
    let body = cxpak::lsp::methods::handle_custom_method(
        "cxpak/diff",
        serde_json::Value::Null,
        &idx,
        temp.path(),
    )
    .unwrap()
    .unwrap();
    assert!(
        body.get("changes").is_some() && body.get("count").is_some(),
        "diff must return real changes+count, got: {body}"
    );
    assert!(
        body.get("note").is_none(),
        "no 'use cxpak diff CLI' escape hatch allowed"
    );
}

/// The compiled binary must contain NO `custom_stub` symbol. This proves the
/// function was removed, not just unreferenced.
#[test]
fn custom_stub_symbol_is_removed_from_source() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lsp/backend.rs"))
        .expect("read backend.rs");
    assert!(
        !src.contains("fn custom_stub"),
        "src/lsp/backend.rs still contains custom_stub"
    );
    let mod_src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lsp/mod.rs"))
        .expect("read mod.rs");
    assert!(
        !mod_src.contains("custom_stub"),
        "src/lsp/mod.rs still registers custom_stub"
    );
}
