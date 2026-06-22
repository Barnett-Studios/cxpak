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

/// Create a throwaway directory with `git init` so drift/diff can run.
fn git_tempdir() -> tempfile::TempDir {
    let temp = tempfile::TempDir::new().unwrap();
    {
        let repo = git2::Repository::init(temp.path()).expect("git2 init");
        std::fs::write(temp.path().join("README.md"), "init\n").unwrap();
        let mut index = repo.index().expect("repo index");
        index
            .add_path(std::path::Path::new("README.md"))
            .expect("git add");
        index.write().expect("index write");
        let tree_oid = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_oid).expect("find tree");
        let sig = git2::Signature::now("t", "t@t").expect("sig");
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .expect("initial commit");
    }
    temp
}

#[test]
fn all_14_lsp_methods_return_non_stub() {
    let idx = make_idx();
    let temp = git_tempdir();
    let root = temp.path();
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
            "cxpak/trace" => serde_json::json!({"symbol": "main"}),
            "cxpak/search" => serde_json::json!({"query": "main"}),
            "cxpak/predict" => serde_json::json!({"files": ["src/main.rs"]}),
            "cxpak/dataFlow" => serde_json::json!({"symbol": "main"}),
            "cxpak/blastRadius" => serde_json::json!({"file": "src/main.rs"}),
            _ => serde_json::Value::Null,
        };
        let result = cxpak::lsp::methods::handle_custom_method(m, params, &idx, root).expect(m);
        let body = result.unwrap_or_else(|| panic!("{m} must return Some"));
        assert!(!is_stub(&body), "{m} returned stub: {body}");
    }
}

// ── Fix 8: cxpak/deadCode returns real dead-code data, not a stub ────────────

#[test]
fn lsp_dead_code_returns_real_data_not_stub() {
    let counter = cxpak::budget::counter::TokenCounter::new();
    let idx = cxpak::index::CodebaseIndex::build_with_content(
        vec![],
        std::collections::HashMap::new(),
        &counter,
        std::collections::HashMap::new(),
    );
    let result = cxpak::lsp::methods::handle_custom_method(
        "cxpak/deadCode",
        serde_json::Value::Null,
        &idx,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .unwrap();
    let status = result.get("status").and_then(|s| s.as_str());
    assert!(
        status != Some("available") && status != Some("not_implemented"),
        "cxpak/deadCode must return real dead-code data, not a stub status: {result}"
    );
    assert!(
        result.get("dead_symbols").is_some(),
        "cxpak/deadCode must include dead_symbols field"
    );
}
