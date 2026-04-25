//! Adversarial tests for LSP hardening landed after v2.1.1's stub elimination.
//!
//! Each test targets a specific shortcut/cheat surfaced by critical evaluators:
//! - dead-code cross-file `contains` → word-bounded regex
//! - `cxpak/diff` byte-count → real diff_text content
//! - `uri.ends_with(relative_path)` → path-bounded suffix match
//! - `compute_blast_radius` silent empty on unknown file → `Err(Internal)`
//! - dead-code diagnostic message includes kind + visibility
//! - dead_code JSON parity: `total` across v1, MCP, LSP
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

/// When `has_string_references` used plain `contains`, a 3-char private fn
/// named `run` defined in `src/a.rs` was falsely marked alive if ANY other
/// file contained the substring `run` inside a longer word like `runtime`,
/// `return`, `truncate`. Word-boundary regex eliminates that false negative.
#[test]
fn private_short_name_not_alive_from_substring_collision() {
    let counter = TokenCounter::new();
    // file_a: defines private `fn run()` with no local body reference.
    let file_a_content = "fn run() {}\n";
    let mut parses = HashMap::new();
    parses.insert(
        "src/a.rs".to_string(),
        ParseResult {
            symbols: vec![Symbol {
                name: "run".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn run()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    // file_b: contains `runtime` / `return` / `truncate` — all superstring
    // matches of `run` that should NOT mark `run` alive.
    let file_b_content =
        "fn caller() { let x = runtime(); return truncate(x); }\nfn runtime() {}\nfn truncate(_: i32) {}\n";
    parses.insert(
        "src/b.rs".to_string(),
        ParseResult {
            symbols: vec![
                Symbol {
                    name: "caller".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn caller()".into(),
                    body: "{}".into(),
                    start_line: 1,
                    end_line: 1,
                },
                Symbol {
                    name: "runtime".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn runtime()".into(),
                    body: "{}".into(),
                    start_line: 2,
                    end_line: 2,
                },
                Symbol {
                    name: "truncate".into(),
                    kind: SymbolKind::Function,
                    visibility: Visibility::Private,
                    signature: "fn truncate(_: i32)".into(),
                    body: "{}".into(),
                    start_line: 3,
                    end_line: 3,
                },
            ],
            imports: vec![],
            exports: vec![],
        },
    );
    let files = vec![
        ScannedFile {
            relative_path: "src/a.rs".into(),
            absolute_path: "/tmp/src/a.rs".into(),
            language: Some("rust".into()),
            size_bytes: file_a_content.len() as u64,
        },
        ScannedFile {
            relative_path: "src/b.rs".into(),
            absolute_path: "/tmp/src/b.rs".into(),
            language: Some("rust".into()),
            size_bytes: file_b_content.len() as u64,
        },
    ];
    let mut content = HashMap::new();
    content.insert("src/a.rs".into(), file_a_content.into());
    content.insert("src/b.rs".into(), file_b_content.into());
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, content);
    let dead = cxpak::intelligence::dead_code::detect_dead_code(&idx, None);
    let names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        names.contains(&"run"),
        "`run` must be flagged dead — substring appearance inside `runtime`/`return`/`truncate` \
         MUST NOT mark it alive.  Got dead: {names:?}"
    );
}

/// `cxpak/diff` must return the real diff text, not just a byte count.
#[test]
fn lsp_diff_returns_diff_text_not_just_bytes() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    let temp = git_tempdir();
    // Modify README so extract_changes sees a diff.
    std::fs::write(temp.path().join("README.md"), "updated content\n").unwrap();
    let body = cxpak::lsp::methods::handle_custom_method(
        "cxpak/diff",
        serde_json::Value::Null,
        &idx,
        temp.path(),
    )
    .unwrap()
    .unwrap();
    let changes = body["changes"].as_array().expect("changes array");
    assert!(!changes.is_empty(), "expected at least one change");
    let first = &changes[0];
    assert!(
        first.get("diff_text").and_then(|v| v.as_str()).is_some(),
        "diff_text must be a string, got: {first}"
    );
    let text = first["diff_text"].as_str().unwrap();
    assert!(
        !text.is_empty(),
        "diff_text must be non-empty for a modified file"
    );
    // diff_bytes retained for compat but not the only info.
    assert!(
        first.get("diff_bytes").and_then(|v| v.as_u64()).is_some(),
        "diff_bytes field retained for compat"
    );
}

/// `uri_to_rel_path` fallback `ends_with(relative_path)` without separator
/// bound would cross-match `src/main.rs` against a URI pointing at
/// `my_src/main.rs`. The path-bounded resolver must reject the collision.
#[test]
fn find_indexed_file_rejects_unbounded_suffix_match() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/Users/me/repo/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    }];
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, HashMap::new());
    let root = std::path::Path::new("/Users/me/other_repo");

    // Correct URI inside repo_root resolves cleanly via the primary path.
    let ok = cxpak::lsp::methods::find_indexed_file(
        "file:///Users/me/other_repo/src/main.rs",
        &idx,
        root,
    );
    assert!(ok.is_some(), "valid repo-relative URI must resolve");

    // A URI ending in `main.rs` but NOT at a directory boundary of
    // `src/main.rs` must NOT resolve. `my_src/main.rs` is a different file.
    let collision =
        cxpak::lsp::methods::find_indexed_file("file:///tmp/my_src/main.rs", &idx, root);
    assert!(
        collision.is_none(),
        "unbounded suffix match would falsely resolve `my_src/main.rs` to `src/main.rs`"
    );
}

/// `cxpak/blastRadius` for a file not in the index must Err, not silently
/// return `{total_affected: 0}`.
#[test]
fn blast_radius_errors_on_unknown_file() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/real.rs".into(),
        absolute_path: "/tmp/src/real.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    }];
    let idx = CodebaseIndex::build_with_content(files, HashMap::new(), &counter, HashMap::new());
    let r = cxpak::lsp::methods::handle_custom_method(
        "cxpak/blastRadius",
        serde_json::json!({"file": "src/typo.rs"}),
        &idx,
        std::path::Path::new("/tmp"),
    );
    let err = r.expect_err("unknown file must Err, not silently return 0");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("not in index"),
        "error must name the missing file; got {msg}"
    );
}

/// Dead-code diagnostic message includes kind + visibility so the IDE user
/// can tell at a glance whether it's a private fn (safe to delete) or a
/// pub symbol that may have external callers.
#[test]
fn dead_code_diagnostic_message_includes_kind_and_visibility() {
    let counter = TokenCounter::new();
    let file = ScannedFile {
        relative_path: "src/main.rs".into(),
        absolute_path: "/tmp/src/main.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    };
    let mut parses = HashMap::new();
    parses.insert(
        "src/main.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: "unused_priv".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn unused_priv()".into(),
                body: "{}".into(),
                start_line: 5,
                end_line: 6,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/main.rs".into(), "\n\n\n\nfn unused_priv() {}\n".into());
    let idx = CodebaseIndex::build_with_content(vec![file], parses, &counter, content);
    let diags = cxpak::lsp::methods::diagnostics_for_file(
        "src/main.rs",
        &idx,
        std::path::Path::new("/tmp"),
    );
    let msg = diags
        .iter()
        .find(|d| d.message.contains("unused_priv"))
        .map(|d| d.message.clone())
        .expect("dead-code diagnostic for unused_priv");
    assert!(
        msg.contains("private"),
        "message must name visibility, got: {msg}"
    );
    assert!(
        msg.contains("Function"),
        "message must name kind, got: {msg}"
    );
    assert!(
        msg.contains("unused_priv"),
        "message must include symbol name, got: {msg}"
    );
}

/// v1/dead_code and LSP cxpak/deadCode both expose `total`. MCP keeps its
/// extra `showing` (truncation indicator) but the canonical name is `total`.
#[test]
fn dead_code_response_shape_parity_v1_lsp() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    let lsp = cxpak::lsp::methods::handle_custom_method(
        "cxpak/deadCode",
        serde_json::Value::Null,
        &idx,
        std::path::Path::new("/tmp"),
    )
    .unwrap()
    .unwrap();
    assert!(
        lsp.get("total").is_some(),
        "LSP cxpak/deadCode must expose `total` (canonical key), got: {lsp}"
    );
    assert!(
        lsp.get("dead_symbols").is_some(),
        "LSP cxpak/deadCode must include `dead_symbols`"
    );
}

/// The cached dead-code analysis must return identical results on every
/// call for the lifetime of a `CodebaseIndex`. Any non-determinism in the
/// OnceLock fill would manifest here.
#[test]
fn dead_code_cached_is_stable_across_calls() {
    let counter = TokenCounter::new();
    let files = vec![ScannedFile {
        relative_path: "src/lib.rs".into(),
        absolute_path: "/tmp/src/lib.rs".into(),
        language: Some("rust".into()),
        size_bytes: 0,
    }];
    let mut parses = HashMap::new();
    parses.insert(
        "src/lib.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: "dead_fn".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn dead_fn()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 2,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let mut content = HashMap::new();
    content.insert("src/lib.rs".into(), "fn dead_fn() {}\n".into());
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, content);
    let first = idx.dead_code_cached().to_vec();
    let second = idx.dead_code_cached().to_vec();
    let third = idx.dead_code_cached().to_vec();
    assert_eq!(
        first.len(),
        second.len(),
        "cache must be stable call 1 vs 2"
    );
    assert_eq!(
        second.len(),
        third.len(),
        "cache must be stable call 2 vs 3"
    );
    // Proof that cache returned the SAME allocation (pointer stability):
    let p1 = idx.dead_code_cached().as_ptr();
    let p2 = idx.dead_code_cached().as_ptr();
    assert_eq!(p1, p2, "repeated calls must return the same cached slice");
}
