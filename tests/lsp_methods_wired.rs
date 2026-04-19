#![cfg(feature = "lsp")]

fn make_idx() -> cxpak::index::CodebaseIndex {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let files = vec![cxpak::scanner::ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 100,
    }];
    let mut c = std::collections::HashMap::new();
    c.insert("src/main.rs".into(), "fn main(){}".into());
    cxpak::index::CodebaseIndex::build_with_content(
        files,
        std::collections::HashMap::new(),
        &counter,
        c,
    )
}

fn is_stub(v: &serde_json::Value) -> bool {
    v.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s == "not_implemented" || s == "available")
        .unwrap_or(false)
}

#[test]
fn all_14_lsp_methods_return_non_stub() {
    let idx = make_idx();
    let methods = [
        // 3 pre-wired (from v1.6.0):
        "cxpak/health",
        "cxpak/conventions",
        "cxpak/blastRadius",
        // 11 newly wired:
        "cxpak/overview",
        "cxpak/trace",
        "cxpak/diff",
        "cxpak/search",
        "cxpak/apiSurface",
        "cxpak/deadCode",
        "cxpak/callGraph",
        "cxpak/predict",
        "cxpak/drift",
        "cxpak/securitySurface",
        "cxpak/dataFlow",
    ];
    for m in methods {
        let params = match m {
            "cxpak/trace" | "cxpak/search" => serde_json::json!({"symbol": "main"}),
            "cxpak/predict" => serde_json::json!({"files": ["src/main.rs"]}),
            "cxpak/dataFlow" => serde_json::json!({"symbol": "main"}),
            _ => serde_json::Value::Null,
        };
        let result = cxpak::lsp::methods::handle_custom_method(m, params, &idx).expect(m);
        let body = result.unwrap_or_else(|| panic!("{m} must return Some"));
        assert!(!is_stub(&body), "{m} returned stub: {body}");
    }
}
