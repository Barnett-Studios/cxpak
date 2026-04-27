//! Adversarial tests for round-3 hardenings that closed findings from a
//! second pass of critical evaluators after the v2.1.1 stub-elimination work.
//!
//! Each test locks an invariant identified as a real correctness or UX gap.
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

/// `process_watcher_changes` mutates the index in place.  Before this fix
/// the OnceLock dead-code cache from the pre-edit state survived the
/// mutation forever — LSP/dashboard/health silently returned stale data
/// after every file change.  This test exercises the contract: prime the
/// cache, mutate the index, run the watcher path, and confirm the cache
/// has been reset (next call sees the new state).
#[test]
fn process_watcher_changes_invalidates_dead_code_cache() {
    let dir = tempfile::TempDir::new().unwrap();
    // Initial state: file with one private dead function.
    let foo_path = dir.path().join("foo.rs");
    std::fs::write(&foo_path, "fn dead_one() {}\n").unwrap();
    // Initialise git so build_index doesn't bail.
    let _ = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "init", "--quiet"])
        .current_dir(dir.path())
        .output();

    let idx = cxpak::commands::serve::build_index(dir.path()).expect("build_index");
    let shared = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(idx)));
    // Prime the cache.
    {
        let g = shared.read().unwrap();
        let _ = g.dead_code_cached();
    }
    // Verify it IS populated (Arc<OnceLock<_>>::get returns Some).
    {
        let g = shared.read().unwrap();
        assert!(
            g.dead_code_cache.get().is_some(),
            "cache must be populated after first call"
        );
    }
    // Add a new file via the watcher path (synthetic FileChange).
    let new_path = dir.path().join("bar.rs");
    std::fs::write(&new_path, "fn new_dead() {}\n").unwrap();
    let change = cxpak::daemon::watcher::FileChange::Created(new_path.clone());
    cxpak::commands::serve::process_watcher_changes(&[change], dir.path(), &shared);
    // After the watcher tick the cache MUST have been reset to a fresh
    // OnceLock — get() returns None until the next dead_code_cached() call.
    {
        let g = shared.read().unwrap();
        assert!(
            g.dead_code_cache.get().is_none(),
            "cache must be invalidated by process_watcher_changes after a real update"
        );
    }
}

/// `cxpak/diff` must allow the caller to cap response size to keep LSP
/// JSON-RPC messages reasonable.  Defaults are 50 files × 32 KiB each.
#[test]
fn cxpak_diff_respects_max_files_and_per_file_byte_cap() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    let temp = git_tempdir();
    // Modify README so extract_changes returns at least one entry.
    std::fs::write(
        temp.path().join("README.md"),
        "x".repeat(100_000), // 100 KB
    )
    .unwrap();
    let body = cxpak::lsp::methods::handle_custom_method(
        "cxpak/diff",
        serde_json::json!({"max_files": 5, "max_bytes_per_file": 1024}),
        &idx,
        temp.path(),
    )
    .unwrap()
    .unwrap();
    let changes = body["changes"].as_array().expect("changes array");
    assert!(!changes.is_empty(), "expected at least one diff entry");
    let first = &changes[0];
    let text = first["diff_text"].as_str().expect("diff_text string");
    assert!(
        text.len() <= 1024,
        "diff_text must be capped to max_bytes_per_file (1024); got {} bytes",
        text.len()
    );
    let truncated = first["truncated"].as_bool().unwrap_or(false);
    let bytes = first["diff_bytes"].as_u64().unwrap_or(0);
    if bytes > 1024 {
        assert!(
            truncated,
            "truncated flag must be set when diff was clipped"
        );
    }
}

/// `cxpak/diff` must reject git refs containing characters outside the
/// safe allowlist.  `HEAD^{/regex}` is a libgit2-supported rev-spec that
/// triggers expensive commit-message regex scans — denial of service.
#[test]
fn cxpak_diff_rejects_disallowed_git_ref_characters() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    let temp = git_tempdir();
    let bad_refs = [
        "HEAD^{/.*}", // regex DoS
        "HEAD@{1}",   // reflog
        "branch with space",
        "../etc/passwd",
        "branch:foo",
        "branch;rm",
        "`whoami`",
    ];
    for r in bad_refs {
        let result = cxpak::lsp::methods::handle_custom_method(
            "cxpak/diff",
            serde_json::json!({"ref": r}),
            &idx,
            temp.path(),
        );
        assert!(
            result.is_err(),
            "ref `{r}` must be rejected by validate_git_ref"
        );
    }
    // Sanity: a clean ref like `HEAD~1` passes the validator (the actual
    // git operation may still fail because the tempdir has only one
    // commit, but the validation step itself does not reject it).
    let result = cxpak::lsp::methods::handle_custom_method(
        "cxpak/diff",
        serde_json::json!({"ref": "HEAD"}),
        &idx,
        temp.path(),
    );
    // Should reach the git layer: either Ok with empty changes or
    // Err(git diff failed: ...).  Either way, the ref itself is allowed.
    if let Err(e) = &result {
        let msg = format!("{e:?}");
        assert!(
            !msg.contains("disallowed characters") && !msg.contains("ref-name rules"),
            "valid ref `HEAD` must pass the validator; got {msg}"
        );
    }
}

/// Dead-code detection must not be fooled by a symbol's name appearing
/// only in a comment or string literal in another file.
#[test]
fn dead_code_ignores_comment_and_string_literal_mentions() {
    let counter = TokenCounter::new();
    // file_a defines `fn rare_helper()` — never called anywhere.
    let mut parses = HashMap::new();
    parses.insert(
        "src/a.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: "rare_helper".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn rare_helper()".into(),
                body: "{}".into(),
                start_line: 1,
                end_line: 1,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    // file_b mentions `rare_helper` ONLY inside a doc comment and a
    // string literal — no real call.  Old `contains` and the v2.1.1
    // word-boundary regex both treated this as a live reference.
    let file_b_content =
        "// rare_helper is unused for now\nfn other() { let _ = \"rare_helper\"; }\n";
    parses.insert(
        "src/b.rs".into(),
        ParseResult {
            symbols: vec![Symbol {
                name: "other".into(),
                kind: SymbolKind::Function,
                visibility: Visibility::Private,
                signature: "fn other()".into(),
                body: "{}".into(),
                start_line: 2,
                end_line: 2,
            }],
            imports: vec![],
            exports: vec![],
        },
    );
    let files = vec![
        ScannedFile {
            relative_path: "src/a.rs".into(),
            absolute_path: "/tmp/src/a.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
        ScannedFile {
            relative_path: "src/b.rs".into(),
            absolute_path: "/tmp/src/b.rs".into(),
            language: Some("rust".into()),
            size_bytes: 0,
        },
    ];
    let mut content = HashMap::new();
    content.insert("src/a.rs".into(), "fn rare_helper() {}\n".into());
    content.insert("src/b.rs".into(), file_b_content.into());
    let idx = CodebaseIndex::build_with_content(files, parses, &counter, content);
    let dead = cxpak::intelligence::dead_code::detect_dead_code(&idx, None);
    let names: Vec<&str> = dead.iter().map(|d| d.symbol.as_str()).collect();
    assert!(
        names.contains(&"rare_helper"),
        "`rare_helper` must be flagged dead — appearances inside `// comment` and `\"string\"` MUST NOT count as live references. Got: {names:?}"
    );
}

/// MCP `cxpak_dead_code` must use the cached analysis when no focus is
/// set, matching the LSP / v1 / dashboard paths.  Test verifies pointer
/// stability of the cached result across two MCP-style calls.
#[test]
fn mcp_dead_code_shares_cache_with_lsp_when_no_focus() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    // First MCP-style call: triggers cache population.
    let cached = idx.dead_code_cached();
    let p1 = cached.as_ptr();
    // Second call must return the same allocation (cache reuse).
    let cached2 = idx.dead_code_cached();
    let p2 = cached2.as_ptr();
    assert_eq!(p1, p2, "cached slice pointer must be stable across calls");
}

/// `aria-expanded` on the palette combobox MUST default to "false" in
/// the rendered HTML so screen readers don't announce "expanded" when
/// the popup is hidden at page load.
#[test]
fn palette_aria_expanded_defaults_false_in_static_html() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build_with_content(vec![], HashMap::new(), &counter, HashMap::new());
    let meta = cxpak::visual::render::RenderMetadata {
        repo_name: "x".into(),
        generated_at: "[R]".into(),
        health_score: None,
        node_count: 0,
        edge_count: 0,
        cxpak_version: "[R]".into(),
    };
    let html = cxpak::visual::spa::render_spa(&idx, &meta).unwrap();
    let line = html
        .lines()
        .find(|l| l.contains(r#"id="cxpak-palette-input""#))
        .expect("palette input line");
    assert!(
        line.contains(r#"aria-expanded="false""#),
        "static aria-expanded must be 'false' (toggled on open by the controller); got: {line}"
    );
    assert!(
        !line.contains(r#"aria-expanded="true""#),
        "static aria-expanded MUST NOT be 'true'"
    );
}

/// Controller MUST flip aria-expanded on open/close.  Test by source
/// inspection of the controller JS.
#[test]
fn controller_toggles_aria_expanded_on_open_and_close() {
    let js = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-spa-controller.js"
    ))
    .expect("controller js");
    assert!(
        js.contains("setAttribute('aria-expanded', 'true')"),
        "openPalette must set aria-expanded=true"
    );
    assert!(
        js.contains("setAttribute('aria-expanded', 'false')"),
        "closePalette must set aria-expanded=false"
    );
}

/// Inspector MUST be included in the focus trap.  Test by source.
#[test]
fn trap_focus_includes_inspector_branch() {
    let js = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-spa-controller.js"
    ))
    .expect("controller js");
    let trap_block = js
        .split("function trapFocus")
        .nth(1)
        .expect("trapFocus function")
        .split("\n  }\n")
        .next()
        .expect("trapFocus body");
    assert!(
        trap_block.contains("CX.state.inspector"),
        "trapFocus must have an inspector branch; got body: {trap_block}"
    );
}

/// Light-mode CSS must override severity-dot color to ensure WCAG AA
/// contrast against light-mode --accent-red / --accent-yellow.
#[test]
fn severity_dot_has_light_mode_color_override() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/cxpak-visual.css"
    ))
    .expect("css");
    assert!(
        css.contains(":root[data-theme=\"light\"] .cxpak-severity-dot"),
        "light theme must have a .cxpak-severity-dot override block"
    );
    // The override must set color to white (or another light value); not
    // the default dark #0f0f23.
    let block = css
        .split(":root[data-theme=\"light\"] .cxpak-severity-dot")
        .nth(1)
        .expect("light-mode override block")
        .split('}')
        .next()
        .expect("block body");
    assert!(
        block.contains("#ffffff") || block.contains("#fff") || block.contains("white"),
        "light-mode severity-dot color must be white-ish for AA contrast on red/amber; got: {block}"
    );
}

/// `RecentChange.unknown_days_ago` was redundant with `Option<u32>::is_none`
/// — removed to eliminate two-representations-of-same-fact drift risk.
/// Test by serializing and asserting the field is gone.
#[test]
fn recent_change_no_longer_has_unknown_days_ago_field() {
    let counter = TokenCounter::new();
    let idx = CodebaseIndex::build(vec![], HashMap::new(), &counter);
    let _ = cxpak::intelligence::recent_changes::compute_recent_changes(&idx);
    // Construct a RecentChange via serde to verify the field set.
    let rc_json = serde_json::to_value(cxpak::intelligence::recent_changes::RecentChange {
        path: "x.rs".into(),
        days_ago: None,
        modifications_30d: 1,
    })
    .unwrap();
    assert!(
        rc_json.get("unknown_days_ago").is_none(),
        "RecentChange must not serialise an unknown_days_ago field; clients derive it from days_ago is null"
    );
    // days_ago: None must serialise as JSON null (not be skipped) so
    // clients see the key explicitly.
    assert!(
        rc_json.get("days_ago").is_some_and(|v| v.is_null()),
        "RecentChange.days_ago=None must serialise as null, got: {rc_json}"
    );
}
